//! `plane_create`: the abort-and-remove lifecycle (ADR-0004).
//!
//! ```text
//! 1. mkdir <plane-dir>            ← the claim, atomic, EEXIST is fatal
//! 2. write .bitplane/incomplete   ← the latch
//! 3. write plane.toml             ← the membership
//! 4. git worktree add, per member ← the abort window
//! 5. unlink .bitplane/incomplete  ← the point of no return
//! ```
//!
//! **The step ordering is the load-bearing part.** Everything expensive or
//! side-effect-bearing happens outside the abort window, so the window holds
//! only cheap, discardable work — which is what makes throw-it-away-and-retry a
//! cheap repair rather than a lossy one, and is why the abort path may pass
//! git's force flag unconditionally. That force is scoped to this path and must
//! not be "consistency-fixed" onto `destroy`, where it would step straight over
//! the uncommitted-work veto.
//!
//! Refusals a preflight can see — a branch already checked out somewhere, a
//! member that is not a repository, an intent that cannot be honoured — are
//! collected **before the claim**, so the common mistake fails with nothing
//! created at all.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::directories::Directories;
use crate::error::{BranchWanted, EngineError};
use crate::interrupt::Interrupt;
use crate::member::{MemberRef, WorktreePath};
use crate::outcome::{Outcome, PerMember, SkipReason};
use crate::plane_dir::ClaimedPlane;
use crate::plane_file::{Member, PlaneFile};
use crate::plane_id::PlaneId;
use crate::repo::{Git, Insistence, canonical_path as canonical};
use crate::wire::{BranchIntent, CreatedMember, PlaneCreateRequest, PlaneCreated};

/// How many worktrees are built at once.
///
/// Per-source-repo locks can never contend within one plane — a plane holds at
/// most one worktree per repository — so the fan-out is genuinely parallel. The
/// cap exists because a plane of twenty members should not fork twenty gits.
const MAX_WORKERS: usize = 4;

/// Everything `plane_create` needs that is not in the request.
pub struct CreateContext<'a> {
    pub directories: &'a Directories,
    pub git: &'a Git,
    /// What a member path's leading `~` means. `None` where the host has no
    /// home directory, which makes `~/…` a path called `~`.
    pub home: Option<&'a Path>,
    pub interrupt: Interrupt,
    /// Called once if the plane's lock is contended, so a caller can say
    /// `waiting for …` without the common case printing anything.
    pub on_lock_wait: &'a (dyn Fn(&Path) + Sync),
}

/// Makes a plane, or makes none.
pub fn plane_create(
    request: &PlaneCreateRequest,
    context: &CreateContext<'_>,
) -> Result<PlaneCreated, EngineError> {
    let planned = plan(request, context)?;

    let claimed = match &request.id {
        Some(id) => ClaimedPlane::claim(context.directories, PlaneId::chosen(id)?)?,
        None => ClaimedPlane::claim_generated(context.directories)?,
    };

    // Held until the plane is complete. On the sentinel, never on `plane.toml`.
    let _lock = claimed.lock(context.on_lock_wait)?;

    claimed.latch()?;
    claimed.write_plane_file(&membership(claimed.id().clone(), &planned))?;

    let members = build(&planned, &claimed, context);
    let interrupted = context.interrupt.is_raised();

    if interrupted || members.iter().any(PerMember::is_failure) {
        return unwind(members, interrupted, &claimed, &planned, context);
    }

    claimed.unlatch()?;

    Ok(PlaneCreated {
        id: claimed.id().clone(),
        directory: claimed.path().to_path_buf(),
        members,
        interrupted: false,
        remnant: false,
    })
}

/// One member, resolved and checked, before anything is created.
struct PlannedMember {
    reference: MemberRef,
    /// The repo the worktree comes from.
    source: PathBuf,
    branch: String,
    /// Where the worktree goes, relative to the plane directory.
    path: WorktreePath,
    /// What a new branch would be cut from. `None` where the branch is already
    /// there and nothing has to be cut.
    base: Option<String>,
    /// What repository the source belongs to. Two members sharing one are two
    /// worktrees of the same thing, however differently they were spelled.
    repository: PathBuf,
}

/// Resolves every member and collects every refusal a preflight can see.
fn plan(
    request: &PlaneCreateRequest,
    context: &CreateContext<'_>,
) -> Result<Vec<PlannedMember>, EngineError> {
    let mut planned: Vec<PlannedMember> = Vec::new();

    for spec in &request.members {
        let member = resolve(spec, request.branch.as_deref(), context)?;

        if let Some(earlier) = planned
            .iter()
            .find(|other| other.reference == member.reference)
        {
            return Err(EngineError::DuplicateMember {
                member: earlier.reference.to_string(),
                plane: None,
            });
        }
        if let Some(earlier) = planned
            .iter()
            .find(|other| other.repository == member.repository)
        {
            return Err(EngineError::SameRepository {
                first: earlier.reference.to_string(),
                second: member.reference.to_string(),
            });
        }
        if let Some(earlier) = planned.iter().find(|other| other.path == member.path) {
            return Err(EngineError::MemberPathCollision {
                first: earlier.reference.to_string(),
                second: member.reference.to_string(),
                path: member.path.to_string(),
            });
        }

        planned.push(member);
    }

    planned
        .iter_mut()
        .try_for_each(|member| preflight(member, request.intent, context.git))?;

    Ok(planned)
}

/// Reads one member as the user wrote it.
///
/// `@name` is a project; anything else is a path to an **ad-hoc member**. The
/// branch is everything after the last colon — git forbids a colon in a ref
/// name, so what *follows* one is unambiguously a branch, but what precedes it
/// is not always unambiguously a whole path.
fn resolve(
    spec: &str,
    fallback_branch: Option<&str>,
    context: &CreateContext<'_>,
) -> Result<PlannedMember, EngineError> {
    let (subject, suffix) = split_branch_suffix(spec);

    let source = match MemberRef::parse(subject)? {
        // A project may now be registered, but a plane cannot hold one yet:
        // resolving `@name` to its source repo, and the branch rules that
        // follow from ownership, are the next slice.
        MemberRef::Project(name) => {
            return Err(EngineError::ProjectNotFound {
                name: name.to_string(),
            });
        }
        MemberRef::Repo(path) => {
            // A path whose last segment contains a colon cannot carry a branch
            // suffix: the split cannot tell the two apart, so bitplane refuses
            // rather than guessing.
            if suffix.is_some() && expand_home(Path::new(spec), context).is_dir() {
                return Err(EngineError::MemberPathAmbiguous {
                    spec: spec.to_owned(),
                });
            }
            // A path member may be relative, absolute, or start with `~`. The
            // canonical, symlink-followed result is what the plane records and
            // what identifies the member afterwards.
            canonicalise(&path, context)?
        }
    };

    let reference = MemberRef::Repo(source.clone());
    let branch = suffix
        .or(fallback_branch)
        .ok_or_else(|| EngineError::BranchUnspecified {
            member: reference.to_string(),
        })?
        .to_owned();

    // Before anything else about the member is asked: everything below assumes
    // there is a repository here.
    context.git.ensure_repository(&source)?;

    Ok(PlannedMember {
        path: reference.worktree_path(&source)?,
        repository: context.git.repository_identity(&source)?,
        reference,
        source,
        branch,
        base: None,
    })
}

/// Everything about one member that can be known before the claim.
fn preflight(
    member: &mut PlannedMember,
    intent: BranchIntent,
    git: &Git,
) -> Result<(), EngineError> {
    let exists = git.branch_exists(&member.source, &member.branch)?;

    match (intent, exists) {
        (BranchIntent::RequireNew, true) => {
            return Err(EngineError::BranchIntentUnmet {
                member: member.reference.to_string(),
                branch: member.branch.clone(),
                wanted: BranchWanted::New,
            });
        }
        (BranchIntent::RequireExisting, false) => {
            return Err(EngineError::BranchIntentUnmet {
                member: member.reference.to_string(),
                branch: member.branch.clone(),
                wanted: BranchWanted::Existing,
            });
        }
        _ => {}
    }

    if exists {
        // Git refuses `worktree add` on a branch checked out in any worktree of
        // the same repo. bitplane says so in its own words rather than passing
        // git's message through.
        if let Some(holder) = git.worktree_holding(&member.source, &member.branch)? {
            return Err(EngineError::BranchOccupied {
                branch: member.branch.clone(),
                repo: member.source.clone(),
                worktree: holder.path,
                stale: holder.prunable,
            });
        }
        return Ok(());
    }

    member.base = Some(git.base_for_a_new_branch(&member.source)?.ok_or_else(|| {
        EngineError::BaseBranchUnresolved {
            member: member.reference.to_string(),
            branch: member.branch.clone(),
        }
    })?);

    Ok(())
}

/// The abort window: one `git worktree add` per member, in parallel.
///
/// "Stop scheduling" is literal — a worker that finds the run aborted or
/// interrupted takes no more members, while the children already running are
/// left to land and are then unwound.
fn build(
    planned: &[PlannedMember],
    claimed: &ClaimedPlane,
    context: &CreateContext<'_>,
) -> Vec<PerMember<CreatedMember>> {
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let rows: Vec<Mutex<Option<Outcome<CreatedMember>>>> =
        planned.iter().map(|_| Mutex::new(None)).collect();

    std::thread::scope(|threads| {
        for _ in 0..planned.len().min(MAX_WORKERS) {
            threads.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some(member) = planned.get(index) else {
                        break;
                    };

                    let outcome = if context.interrupt.is_raised() {
                        Outcome::Skipped(SkipReason::Interrupted)
                    } else if stop.load(Ordering::SeqCst) {
                        Outcome::Skipped(SkipReason::AbortedAfterEarlierFailure)
                    } else {
                        match add_worktree(member, claimed.path(), context.git) {
                            Ok(created) => Outcome::Ok(created),
                            Err(error) => {
                                stop.store(true, Ordering::SeqCst);
                                Outcome::Failed(error)
                            }
                        }
                    };

                    *rows[index].lock().expect("no worker panics holding this") = Some(outcome);
                }
            });
        }
    });

    planned
        .iter()
        .zip(rows)
        .map(|(member, row)| {
            let outcome = row
                .into_inner()
                .expect("no worker panics holding this")
                .expect("every member is either built or skipped");
            PerMember::new(member.reference.clone(), outcome)
        })
        .collect()
}

fn add_worktree(
    member: &PlannedMember,
    plane_directory: &Path,
    git: &Git,
) -> Result<CreatedMember, EngineError> {
    let at = member.path.under(plane_directory);

    if let Some(parent) = at.parent() {
        std::fs::create_dir_all(parent).map_err(|err| EngineError::io(parent, err))?;
    }

    match &member.base {
        Some(base) => {
            git.add_worktree_on_a_new_branch(&member.source, &at, &member.branch, base)?
        }
        None => git.add_worktree(&member.source, &at, &member.branch)?,
    }

    Ok(CreatedMember {
        branch: member.branch.clone(),
        created_branch: member.base.is_some(),
        path: member.path.clone(),
        unwound: false,
    })
}

/// Takes back everything this run put in a source repo, then the plane
/// directory.
///
/// Driven from what the **source repos** say, not from the rows: a real Ctrl-C
/// reaches the whole process group, so a `git worktree add` can be killed after
/// registering its admin entry and before answering. That member's row is
/// `Failed`, and unwinding only the `Ok` rows would leave the entry — and the
/// branch — behind in a repo bitplane does not own, pointing into a plane
/// directory about to be deleted.
///
/// An interrupted run still answers `Ok`: the rows are what happened, and the
/// exit code says it was interrupted. A failed one answers `Err`, because the
/// envelope `Err` is for an operation that produced no durable state — "never
/// started" and "started, then fully unwound" alike.
fn unwind(
    members: Vec<PerMember<CreatedMember>>,
    interrupted: bool,
    claimed: &ClaimedPlane,
    planned: &[PlannedMember],
    context: &CreateContext<'_>,
) -> Result<PlaneCreated, EngineError> {
    let mut rollback: Vec<PerMember<()>> = Vec::new();
    let mut members = members;

    for (member, row) in planned.iter().zip(members.iter_mut()) {
        match take_back(member, claimed.path(), context.git) {
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

    let discarded = rollback.iter().all(|row| !row.is_failure()) && claimed.discard().is_ok();

    if interrupted {
        return Ok(PlaneCreated {
            id: claimed.id().clone(),
            directory: claimed.path().to_path_buf(),
            members,
            interrupted: true,
            remnant: !discarded,
        });
    }

    Err(EngineError::CreateAborted {
        id: claimed.id().to_string(),
        members,
        rollback,
        remnant: !discarded,
    })
}

/// Removes whatever this run left in one member's source repo, and says whether
/// there was anything to remove.
///
/// The worktree first, then the branch — `git branch -D` refuses a branch some
/// worktree still holds, and the order is also the one that leaves nothing
/// dangling if the second step fails.
fn take_back(
    member: &PlannedMember,
    plane_directory: &Path,
    git: &Git,
) -> Result<bool, EngineError> {
    let mut took_anything = false;

    // Matched on git's own recorded path rather than on the one bitplane passed
    // in: git canonicalises at `worktree add` time, so `/tmp/…` comes back as
    // `/private/tmp/…` on macOS and the two never compare equal.
    let inside = canonical(plane_directory);
    for entry in git.worktrees(&member.source)? {
        if !canonical(&entry.path).starts_with(&inside) {
            continue;
        }

        // Forced unconditionally, and only here: by construction nothing in the
        // abort window is the user's yet.
        if let Err(refused) = git.remove_worktree(&member.source, &entry.path, Insistence::Forced) {
            // An entry whose directory something already deleted is prune's
            // business and nothing else's — and prune skips a locked worktree,
            // so the entry has to be confirmed gone rather than assumed.
            if !entry.prunable {
                return Err(refused);
            }
            git.prune_worktrees(&member.source)?;
            if still_registered(&entry.path, &member.source, git)? {
                return Err(refused);
            }
        }
        took_anything = true;
    }

    // A branch is this run's only if this run had to cut it — a branch that
    // already existed at the preflight is the user's, and is left alone.
    if member.base.is_some() && git.branch_exists(&member.source, &member.branch)? {
        git.delete_branch(&member.source, &member.branch)?;
        took_anything = true;
    }

    Ok(took_anything)
}

fn still_registered(at: &Path, repo: &Path, git: &Git) -> Result<bool, EngineError> {
    Ok(git.worktrees(repo)?.iter().any(|entry| entry.path == at))
}

fn membership(id: PlaneId, planned: &[PlannedMember]) -> PlaneFile {
    PlaneFile::new(
        id,
        planned
            .iter()
            .map(|member| Member {
                path: member.path.clone(),
                source: member.reference.clone(),
            })
            .collect(),
    )
}

/// The branch is everything after the **last** colon, where there is one.
fn split_branch_suffix(spec: &str) -> (&str, Option<&str>) {
    match spec.rsplit_once(':') {
        Some((subject, branch)) if !subject.is_empty() && !branch.is_empty() => {
            (subject, Some(branch))
        }
        _ => (spec, None),
    }
}

/// Expands a leading `~`, then canonicalises — following symlinks, so a member
/// reached two ways is one member.
fn canonicalise(path: &Path, context: &CreateContext<'_>) -> Result<PathBuf, EngineError> {
    let expanded = expand_home(path, context);

    expanded
        .canonicalize()
        .map_err(|_| EngineError::MemberNotARepository {
            path: expanded.clone(),
            reason: if expanded.exists() {
                "could not be resolved".to_owned()
            } else {
                "does not exist".to_owned()
            },
        })
}

/// A leading `~`, replaced by the home directory the engine was built with.
fn expand_home(path: &Path, context: &CreateContext<'_>) -> PathBuf {
    match (path.strip_prefix("~"), context.home) {
        (Ok(rest), Some(home)) => home.join(rest),
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_branch_suffix_is_everything_after_the_last_colon() {
        let cases = [
            ("@api", ("@api", None)),
            ("@api:feat-login", ("@api", Some("feat-login"))),
            (
                "~/projects/bitplane:feat-x",
                ("~/projects/bitplane", Some("feat-x")),
            ),
            (
                "~/projects/bitplane:feat/x",
                ("~/projects/bitplane", Some("feat/x")),
            ),
            ("/repos/api", ("/repos/api", None)),
            // Nothing on one side of the colon is not a suffix.
            ("/repos/api:", ("/repos/api:", None)),
        ];

        for (spec, expected) in cases {
            assert_eq!(split_branch_suffix(spec), expected, "for {spec}");
        }
    }
}
