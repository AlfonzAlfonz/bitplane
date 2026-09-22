//! `plane_add`: putting a member into a plane that already exists.
//!
//! ```text
//! 1. resolve the plane, read its membership   ← nothing is touched yet
//! 2. fetch every owned project being added
//! 3. lock the plane, re-read the membership
//! 4. append the new entries to plane.toml
//! 5. git worktree add, per member
//! 6. on failure: force-remove only what this run made, drop only its entries
//! ```
//!
//! **`add` cannot abort-and-remove the way `create` does**, because the plane
//! holds other members full of work: there is no directory to discard, so the
//! unwind is surgical rather than wholesale. It removes only the worktrees this
//! run created, deletes only the branches this run cut, and drops only its own
//! entries — leaving the plane exactly as it found it.
//!
//! **`add` never sets the incomplete latch, under any failure.** The latch means
//! *this plane was never completed, nothing in it is yours*, and that is what
//! licenses `destroy` running no refusal checks at all on a latched plane. A
//! plane full of real work flagged as free to discard would be a trapdoor —
//! which is also why `add` **declines** on a plane that is already latched
//! rather than adding to it.
//!
//! The entries are written **before** the worktrees, the same order `create`
//! uses and for the same reason: the window in which a member is visible
//! strictly contains the window in which its worktree exists, so an unwind that
//! itself dies leaves a *listed* member with no worktree — a finding on
//! `bp show`, cleared by `bp rm` — rather than an unlisted worktree nothing can
//! see.

use std::path::Path;

use crate::directories::Directories;
use crate::error::EngineError;
use crate::interrupt::Interrupt;
use crate::outcome::{Outcome, PerMember};
use crate::plan::{self, PlanContext, PlannedMember};
use crate::plane_dir::OpenPlane;
use crate::plane_file::{self, Member, PlaneFile};
use crate::project_fetch::{self, FetchContext};
use crate::read;
use crate::repo::Git;
use crate::wire::{CreatedMember, PlaneAddRequest, PlaneAdded, ProjectFetchRequest};
use crate::worktrees;

/// Everything `plane_add` needs that is not in the request.
pub struct AddContext<'a> {
    pub directories: &'a Directories,
    pub git: &'a Git,
    /// What a member path's leading `~` means.
    pub home: Option<&'a Path>,
    pub interrupt: Interrupt,
    pub on_lock_wait: &'a (dyn Fn(&Path) + Sync),
}

/// Puts the members named into a plane, or puts none and leaves it as it was.
pub fn plane_add(
    request: &PlaneAddRequest,
    context: &AddContext<'_>,
) -> Result<PlaneAdded, EngineError> {
    crate::create::validate(
        request.intent,
        request.fetch,
        request.branch.as_deref(),
        &request.members,
    )?;

    let plane = OpenPlane::at(read::resolve(&request.plane, context.directories)?);

    // Before the lock and before the fetch: a typo, a duplicate, or a plane
    // that is on its way to being discarded costs nothing to notice, and a
    // forge round trip is the most expensive thing on this path.
    let mut planned = resolve_members(request, &plane, context)?;

    if request.fetch {
        fetch_first(&plan::fetchable(&planned), context)?;
    }

    let _lock = plane.lock(context.on_lock_wait)?;

    // Re-checked under the lock, against the file as it is now: the membership
    // read above was read without one, and two `bp add`s racing on one plane is
    // exactly what the lock is for.
    let existing = membership(&plane)?;
    refuse_collisions(&planned, &existing, &plane, context)?;
    plan::preflight_all(&mut planned, request.intent, context.git)?;

    plane_file_with(&plane, &entries(&planned))?;

    let members = worktrees::build(&planned, plane.path(), context.git, context.interrupt);
    let interrupted = context.interrupt.is_raised();

    if interrupted || members.iter().any(PerMember::is_failure) {
        return unwind(members, interrupted, &plane, &planned, context);
    }

    Ok(PlaneAdded {
        id: plane.id().to_owned(),
        directory: plane.path().to_path_buf(),
        members,
        interrupted: false,
        remnant: false,
    })
}

/// Reads the members as written, and refuses one the plane already holds.
///
/// A latched plane is declined here rather than added to: whatever `add` put
/// into one would be flagged as free to discard by a marker that only ever told
/// the truth about a plane nothing had used.
fn resolve_members(
    request: &PlaneAddRequest,
    plane: &OpenPlane,
    context: &AddContext<'_>,
) -> Result<Vec<PlannedMember>, EngineError> {
    if plane.is_latched() {
        return Err(EngineError::PlaneIncomplete {
            id: plane.id().to_owned(),
        });
    }

    let existing = membership(plane)?;
    let planned = plan::resolve_all(
        &request.members,
        request.branch.as_deref(),
        &context.planning(),
    )?;

    refuse_collisions(&planned, &existing, plane, context)?;

    Ok(planned)
}

/// Every way a member being added would break the plane's one-worktree-per-
/// project rule, checked against what the plane already holds.
///
/// Adding a member the plane already has is a **typed duplicate error** and not
/// an adoption: `add` will not quietly take over a listed member whose worktree
/// has gone missing. The fix is two commands, deliberately, because that makes
/// the removal visible instead of implied.
fn refuse_collisions(
    planned: &[PlannedMember],
    existing: &[Member],
    plane: &OpenPlane,
    context: &AddContext<'_>,
) -> Result<(), EngineError> {
    for member in planned {
        if existing.iter().any(|held| held.source == member.reference) {
            return Err(EngineError::DuplicateMember {
                member: member.reference.to_string(),
                plane: Some(plane.id().to_owned()),
            });
        }
        if let Some(held) = existing.iter().find(|held| held.path == member.path) {
            return Err(EngineError::MemberPathCollision {
                first: held.source.to_string(),
                second: member.reference.to_string(),
                path: member.path.to_string(),
            });
        }
    }

    // One `git rev-parse` per member the plane already holds, and only for the
    // ones whose repo is still there. Two members that are different paths but
    // one repository derive the same worktree path only by luck, so the path
    // check above cannot stand in for this one.
    for held in existing {
        let source = plan::source_of(&held.source, context.directories);
        let Some(repo) = source.found() else {
            continue;
        };
        let Ok(repository) = context.git.repository_identity(repo) else {
            continue;
        };

        if let Some(member) = planned
            .iter()
            .find(|member| member.repository == repository)
        {
            return Err(EngineError::SameRepository {
                first: held.source.to_string(),
                second: member.reference.to_string(),
            });
        }
    }

    Ok(())
}

/// Brings every owned project being added up to date, or fails naming the ones
/// that would not answer.
///
/// Before the plane's lock, not under it: a fetch is a forge round trip, and
/// holding a plane's lock across one would block every other `bp` aimed at that
/// plane for as long as the network takes.
fn fetch_first(projects: &[String], context: &AddContext<'_>) -> Result<(), EngineError> {
    if projects.is_empty() {
        return Ok(());
    }

    let fetched = project_fetch::project_fetch(
        &ProjectFetchRequest {
            projects: projects.to_vec(),
        },
        &FetchContext {
            directories: context.directories,
            git: context.git,
            interrupt: context.interrupt,
            on_lock_wait: context.on_lock_wait,
        },
    )?;

    match fetched.failure() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Takes back only what this run made, then drops only this run's entries.
///
/// An interrupted run still answers `Ok`, for the reason `create`'s does: the
/// rows are what happened, and the exit code says it was interrupted. A failed
/// one answers `Err`, because the envelope `Err` is for an operation that
/// produced no durable state — and a plane returned to its prior state produced
/// none.
fn unwind(
    members: Vec<PerMember<CreatedMember>>,
    interrupted: bool,
    plane: &OpenPlane,
    planned: &[PlannedMember],
    context: &AddContext<'_>,
) -> Result<PlaneAdded, EngineError> {
    let mut rollback: Vec<PerMember<()>> = Vec::new();
    let mut members = members;

    for (member, row) in planned.iter().zip(members.iter_mut()) {
        match worktrees::take_back(member, plane.path(), context.git) {
            // Nothing of this member's ever reached its repo.
            Ok(false) => {}
            Ok(true) => {
                if let Outcome::Ok(created) = &mut row.outcome {
                    created.unwound = true;
                }
                rollback.push(PerMember::ok(member.reference.clone(), ()));
            }
            Err(error) => rollback.push(PerMember::failed(member.reference.clone(), error)),
        }
    }

    let restored =
        rollback.iter().all(|row| !row.is_failure()) && plane_file_without(plane, planned).is_ok();

    if interrupted {
        return Ok(PlaneAdded {
            id: plane.id().to_owned(),
            directory: plane.path().to_path_buf(),
            members,
            interrupted: true,
            remnant: !restored,
        });
    }

    Err(EngineError::AddAborted {
        plane: plane.id().to_owned(),
        members,
        rollback,
        remnant: !restored,
    })
}

/// The plane's membership, which must be there: a directory with no plane file
/// is a claim `create` never finished, and `add` has already declined that.
fn membership(plane: &OpenPlane) -> Result<Vec<Member>, EngineError> {
    plane
        .read_plane_file()?
        .map(|file: PlaneFile| file.members)
        .ok_or_else(|| EngineError::PlaneIncomplete {
            id: plane.id().to_owned(),
        })
}

fn entries(planned: &[PlannedMember]) -> Vec<Member> {
    planned
        .iter()
        .map(|member| Member {
            path: member.path.clone(),
            source: member.reference.clone(),
        })
        .collect()
}

/// Appends the new entries, preserving everything else the file holds — a
/// user's `# do not reap, long-running migration` included.
fn plane_file_with(plane: &OpenPlane, added: &[Member]) -> Result<(), EngineError> {
    let path = plane.plane_file_path();
    let text = std::fs::read_to_string(&path).map_err(|err| EngineError::io(&path, err))?;

    plane.write_plane_file_text(&plane_file::with(&path, &text, added)?)
}

/// Drops this run's entries and nobody else's, keyed by the worktree path the
/// file names them with.
fn plane_file_without(plane: &OpenPlane, planned: &[PlannedMember]) -> Result<(), EngineError> {
    let path = plane.plane_file_path();
    let text = std::fs::read_to_string(&path).map_err(|err| EngineError::io(&path, err))?;
    let ours: Vec<crate::member::WorktreePath> =
        planned.iter().map(|member| member.path.clone()).collect();

    plane.write_plane_file_text(&plane_file::without(&path, &text, &ours)?)
}

impl AddContext<'_> {
    /// What member resolution needs out of this, and nothing else.
    fn planning(&self) -> PlanContext<'_> {
        PlanContext {
            directories: self.directories,
            git: self.git,
            home: self.home,
        }
    }
}
