//! Building a plane's worktrees, and taking back the ones a failed run made.
//!
//! Shared by `create` and `add`, which differ in what they do *around* the
//! build — one claims a directory and may discard it, the other writes into a
//! plane full of somebody's work — and not at all in the build itself. One
//! `git worktree add` per member, in parallel, stopping the moment a member
//! fails.
//!
//! The force flag [`take_back`] passes is scoped to exactly this path and must
//! not be "consistency-fixed" onto `destroy`, where it would step straight over
//! the uncommitted-work veto (ADR-0004). It is licensed here because every
//! worktree it removes is one this same run created seconds earlier, from a
//! branch that either did not exist or was left untouched.

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::error::EngineError;
use crate::interrupt::Interrupt;
use crate::outcome::{Outcome, PerMember, SkipReason};
use crate::plan::{Base, PlannedMember};
use crate::repo::{Git, Insistence, canonical_path as canonical};
use crate::wire::CreatedMember;

/// How many worktrees are built at once.
///
/// Per-source-repo locks can never contend within one plane — a plane holds at
/// most one worktree per repository — so the fan-out is genuinely parallel. The
/// cap exists because a plane of twenty members should not fork twenty gits.
const MAX_WORKERS: usize = 4;

/// One `git worktree add` per member, in parallel, in the order they were
/// named.
///
/// "Stop scheduling" is literal — a worker that finds the run aborted or
/// interrupted takes no more members, while the children already running are
/// left to land and are then unwound.
pub fn build(
    planned: &[PlannedMember],
    plane_directory: &Path,
    git: &Git,
    interrupt: Interrupt,
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

                    let outcome = if interrupt.is_raised() {
                        Outcome::Skipped(SkipReason::Interrupted)
                    } else if stop.load(Ordering::SeqCst) {
                        Outcome::Skipped(SkipReason::AbortedAfterEarlierFailure)
                    } else {
                        match add_worktree(member, plane_directory, git) {
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

/// Removes whatever this run left in one member's source repo, and says whether
/// there was anything to remove.
///
/// Driven from what the **source repo** says rather than from the row: a real
/// Ctrl-C reaches the whole process group, so a `git worktree add` can be killed
/// after registering its admin entry and before answering. That member's row is
/// `Failed`, and unwinding only the `Ok` rows would leave the entry — and the
/// branch — behind in a repo pointing into a plane directory about to be
/// emptied.
///
/// The worktree first, then the branch — `git branch -D` refuses a branch some
/// worktree still holds, and the order is also the one that leaves nothing
/// dangling if the second step fails.
pub fn take_back(
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
        Some(base) => git.add_worktree_on_a_new_branch(
            &member.source,
            &at,
            &member.branch,
            base.as_ref_name(),
        )?,
        None => git.add_worktree(&member.source, &at, &member.branch)?,
    }

    Ok(CreatedMember {
        branch: member.branch.clone(),
        // A branch cut from the forge's own is one that already existed: the
        // local ref is new, the *branch* is not, and the row marks the
        // difference between joining a colleague's work and starting your own.
        created_branch: member.base.as_ref().is_some_and(Base::is_fresh),
        path: member.path.clone(),
        unwound: false,
    })
}

fn still_registered(at: &Path, repo: &Path, git: &Git) -> Result<bool, EngineError> {
    Ok(git.worktrees(repo)?.iter().any(|entry| entry.path == at))
}
