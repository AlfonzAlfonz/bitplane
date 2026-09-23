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
//!
//! The **fetch is outside the window too**, before step 1: it writes the source
//! repo rather than the plane, so a retry finds it warm, and the branch the
//! user asked for has to resolve against what the forge actually has rather
//! than against whatever this host last saw.
//!
//! **`post_worktree_create` scripts run after step 5**, past the point of no
//! return, which is what makes ADR-0004's *"a script failure never unwinds
//! anything"* true by construction: by then there is nothing left to stop. The
//! plane is kept and the run exits `1`, because exiting `0` would hide a real
//! failure.

use std::path::Path;

use crate::directories::Directories;
use crate::error::EngineError;
use crate::interrupt::Interrupt;
use crate::outcome::{Outcome, PerMember};
use crate::plan::{self, PlanContext, PlannedMember};
use crate::plane_dir::ClaimedPlane;
use crate::plane_file::{Member, PlaneFile};
use crate::plane_id::PlaneId;
use crate::project_fetch::{self, FetchContext};
use crate::repo::Git;
use crate::scripts::{self, InPlane, ScriptContext, ScriptPoint, Stop};
use crate::wire::{
    BranchIntent, CreatedMember, PlaneCreateRequest, PlaneCreated, ProjectFetchRequest,
    ScriptOutcome,
};
use crate::worktrees;

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
    /// Where merged script output goes as it arrives.
    pub on_script_output: &'a (dyn Fn(&[u8]) + Sync),
}

impl CreateContext<'_> {
    /// What member resolution needs out of this, and nothing else.
    pub fn planning(&self) -> PlanContext<'_> {
        PlanContext {
            directories: self.directories,
            git: self.git,
            home: self.home,
        }
    }

    /// What running a script needs out of this, and nothing else.
    pub fn scripting(&self) -> ScriptContext<'_> {
        ScriptContext {
            directories: self.directories,
            interrupt: self.interrupt,
            output: self.on_script_output,
        }
    }
}

/// Makes a plane, or makes none.
pub fn plane_create(
    request: &PlaneCreateRequest,
    context: &CreateContext<'_>,
) -> Result<PlaneCreated, EngineError> {
    validate(
        request.intent,
        request.fetch,
        request.branch.as_deref(),
        &request.members,
    )?;

    let planned = plan(request, context)?;

    let claimed = match &request.id {
        Some(id) => ClaimedPlane::claim(context.directories, PlaneId::chosen(id)?)?,
        None => ClaimedPlane::claim_generated(context.directories)?,
    };

    // Held until the plane is complete. On the sentinel, never on `plane.toml`.
    let lock = claimed.lock(context.on_lock_wait)?;

    claimed.latch()?;
    claimed.write_plane_file(&membership(claimed.id().clone(), &planned))?;

    let members = worktrees::build(&planned, claimed.path(), context.git, context.interrupt);
    let interrupted = context.interrupt.is_raised();

    if interrupted || members.iter().any(PerMember::is_failure) {
        return unwind(members, interrupted, &claimed, &planned, context);
    }

    claimed.unlatch()?;

    // **Released before the scripts run**, not after. The plane is complete by
    // here and no script touches `plane.toml`, while a script has no timeout by
    // design — so holding it across a `pnpm i && pnpm build` would make every
    // other `bp` aimed at this plane wait for the build. It is the same
    // reasoning that gives `bp run` no lock at all.
    drop(lock);

    // After the latch clears, so a script failure has nothing left to unwind.
    let scripts = match request.run_scripts {
        true => scripts::at_point(
            ScriptPoint::PostWorktreeCreate,
            &entries(&planned),
            InPlane {
                id: claimed.id().as_str(),
                directory: claimed.path(),
            },
            // Post-scripts do not short-circuit: by construction there is
            // nothing left to stop, so every member still gets its setup.
            Stop::Never,
            &context.scripting(),
        )?,
        false => Vec::new(),
    };

    if let Some(failed) = script_failure(&scripts) {
        return Err(failed);
    }

    Ok(PlaneCreated {
        id: claimed.id().clone(),
        directory: claimed.path().to_path_buf(),
        members,
        interrupted: context.interrupt.is_raised(),
        remnant: false,
        scripts,
    })
}

/// What a `post_worktree_create` pass reports, where any of it did not work.
///
/// The plane is **kept**: scripts run past the point of no return, so a failure
/// there unwinds nothing — but exiting `0` would hide it (ADR-0004). Shared
/// with `add`, which is the identical situation seen twice.
pub fn script_failure(scripts: &[ScriptOutcome]) -> Option<EngineError> {
    scripts
        .iter()
        .any(|outcome| !outcome.succeeded())
        .then(|| EngineError::ScriptFailed {
            outcomes: scripts.to_vec(),
            worktree_created: true,
        })
}

/// Resolves every member, fetches what the branch must resolve against, and
/// collects every refusal a preflight can see.
///
/// The fetch sits between the two halves deliberately: a typo or a duplicate
/// fails before a single forge is contacted, and every branch question is asked
/// afterwards, against a source repo that is up to date.
fn plan(
    request: &PlaneCreateRequest,
    context: &CreateContext<'_>,
) -> Result<Vec<PlannedMember>, EngineError> {
    let mut planned = plan::resolve_all(
        &request.members,
        request.branch.as_deref(),
        &context.planning(),
    )?;

    if request.fetch {
        fetch_first(&plan::fetchable(&planned), context)?;
    }

    plan::preflight_all(&mut planned, request.intent, context.git)?;

    Ok(planned)
}

/// Brings every owned project named up to date, or fails naming the ones that
/// would not answer.
///
/// Reuses `project_fetch`'s fan-out rather than fetching inline: the per-project
/// locks, the parallelism and the per-row failures are already there, and a
/// second implementation would be a second answer to *what does a fetch of this
/// project do*.
///
/// A forge that will not answer stops the run **here**, before the claim, so
/// nothing is created at all. Carrying on would mean resolving the branch
/// against a stale source repo, which is the exact silence
/// `branch_intent_requires_fetch` exists to prevent.
fn fetch_first(projects: &[String], context: &CreateContext<'_>) -> Result<(), EngineError> {
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

/// What a request asks for that cannot be honoured whatever is on disk.
///
/// The one rule here is ADR-0007's: [`BranchIntent::Resolve`] creates a branch
/// when it does not resolve, so switching the fetch off turns
/// `bp create @api:colleagues-branch --no-fetch` against a stale source repo
/// into a new, unrelated branch of that name — found out at push time, having
/// already committed. Spelled as a validation on the **request** rather than as
/// a behaviour change inside `Resolve`, so `BranchIntent` stays a pure
/// statement of intent.
pub fn validate(
    intent: BranchIntent,
    fetch: bool,
    branch: Option<&str>,
    members: &[String],
) -> Result<(), EngineError> {
    if intent == BranchIntent::Resolve && !fetch {
        return Err(EngineError::BranchIntentRequiresFetch {
            branch: branch_named(branch, members),
        });
    }

    Ok(())
}

/// The branch the remedy names: the one `-b` gave, or the first suffix a member
/// carried. `None` where the request named none at all, which is a different
/// failure the caller has not reached yet.
fn branch_named(branch: Option<&str>, members: &[String]) -> Option<String> {
    branch.map(str::to_owned).or_else(|| {
        members
            .iter()
            .find_map(|spec| plan::split_branch_suffix(spec).1)
            .map(str::to_owned)
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
        match worktrees::take_back(member, claimed.path(), context.git) {
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
            scripts: Vec::new(),
        });
    }

    Err(EngineError::CreateAborted {
        id: claimed.id().to_string(),
        members,
        rollback,
        remnant: !discarded,
    })
}

fn membership(id: PlaneId, planned: &[PlannedMember]) -> PlaneFile {
    PlaneFile::new(id, entries(planned))
}

/// The plane file's own view of what was planned, which is also what a script
/// pass fans out over — so the two can never disagree about member order.
fn entries(planned: &[PlannedMember]) -> Vec<Member> {
    planned
        .iter()
        .map(|member| Member {
            path: member.path.clone(),
            source: member.reference.clone(),
        })
        .collect()
}
