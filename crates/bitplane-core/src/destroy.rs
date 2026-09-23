//! `plane_destroy` and `plane_remove`: the converging teardown (ADR-0004).
//!
//! ```text
//! 1. open the plane, take its lock
//! 2. check every member, collect every refusal   ← nothing is touched yet
//! 3. pre_worktree_remove scripts, one sequential pass  ← still nothing removed
//! 4. git worktree remove, per member
//! 5. unlink plane.toml        (destroy)  |  rewrite plane.toml  (rm, last)
//! 6. rm -r the plane directory (destroy)
//! ```
//!
//! **Step 2 finishes before step 3 begins.** Every reason across every member
//! comes back in one error, because a half-destroyed plane is worse than a
//! refused one — and because that is what lets one re-run with the right
//! waivers finish the job.
//!
//! **Scripts cannot join that pass**, and run only once every refusal has
//! passed: running `docker compose down` and *then* refusing the destroy over
//! uncommitted work would leave the containers stopped and the plane intact.
//! A pre-script that exits non-zero **blocks** — nothing is removed — which is
//! its own error and not a refusal, because a script's exit code is a program's
//! opinion rather than work the user is looking at. `--no-scripts` is on the
//! request, so a `project.toml` can never make a plane undestroyable
//! (ADR-0007).
//!
//! **It converges rather than rolling back.** You cannot un-remove a worktree,
//! so an interrupted or half-failed run is finished by running it again:
//! refusals are re-checked against what survived and the members already gone
//! are reported `AlreadyDone`.
//!
//! `plane.toml` is the **first thing written** by `create` and the **last thing
//! removed** here, so the window in which a plane is visible strictly contains
//! the window in which it exists. Anything that exists can be found and named.
//!
//! Nothing on this path passes git's force flag unconditionally. ADR-0004
//! scoped that to `create`'s abort window, where by construction nothing is the
//! user's yet; here each `--force` is unlocked by a waiver the user granted for
//! something this very member has.
//!
//! **The branch goes with the worktree, for an owned project and nothing else.**
//! For a bitplane-owned source repo `refs/heads/*` holds exactly the branches
//! plane members were created on, so the branch is the plane's to delete — and
//! it needs no extra waiver and no second gate, because the `unpushed` check
//! above has already run and a branch that got this far is provably recoverable
//! from the forge. For an **adopted** project or an **ad-hoc member** no ref is
//! touched at all: bitplane cannot tell a branch it created from one the user
//! has had for two years in a repo it merely pointed at (ADR-0006).

use std::path::{Path, PathBuf};

use crate::directories::Directories;
use crate::error::{EngineError, Teardown};
use crate::interrupt::Interrupt;
use crate::member::{MemberRef, WorktreePath};
use crate::outcome::{Outcome, PerMember, SkipReason};
use crate::plan;
use crate::plane_dir::OpenPlane;
use crate::plane_file::{self, Member, PlaneFile};
use crate::read;
use crate::refusal::{Reason, Refusal, Waivers};
use crate::repo::{Git, Insistence, WorkingTree, WorktreeLock, canonical_path};
use crate::scripts::{self, InPlane, ScriptContext, ScriptPoint, Stop};
use crate::wire::{
    BranchDisposition, PlaneDestroyRequest, PlaneDestroyed, PlaneRemoveRequest, PlaneRemoved,
    RemovedMember, ScriptOutcome,
};

/// Everything the teardown verbs need that is not in the request.
pub struct TeardownContext<'a> {
    pub directories: &'a Directories,
    pub git: &'a Git,
    /// What a member path's leading `~` means.
    pub home: Option<&'a Path>,
    pub interrupt: Interrupt,
    pub on_lock_wait: &'a (dyn Fn(&Path) + Sync),
    /// Where merged script output goes as it arrives.
    pub on_script_output: &'a (dyn Fn(&[u8]) + Sync),
}

impl TeardownContext<'_> {
    /// What running a script needs out of this, and nothing else.
    fn scripting(&self) -> ScriptContext<'_> {
        ScriptContext {
            directories: self.directories,
            interrupt: self.interrupt,
            output: self.on_script_output,
        }
    }
}

/// Takes a plane apart, or refuses and takes nothing.
pub fn plane_destroy(
    request: &PlaneDestroyRequest,
    context: &TeardownContext<'_>,
) -> Result<PlaneDestroyed, EngineError> {
    let plane = OpenPlane::at(read::resolve(&request.plane, context.directories)?);
    let _lock = plane.lock(context.on_lock_wait)?;

    // A latched plane is one `create` claimed and never finished, so nothing in
    // it was ever the user's: no script has run, so there are no untracked
    // files, and the branches are seconds old. Every refusal reason is
    // structurally impossible, which is what licenses skipping the pass
    // entirely — and what makes `destroy` the one way out of a latch.
    let incomplete = plane.is_latched();
    let membership = plane.read_plane_file()?;
    let members = membership.map(|file| file.members).unwrap_or_default();

    let waivers = Waivers::new(request.waive.clone());
    let checked = if incomplete {
        // Only where each worktree is and whether anything of it survives. The
        // contents questions have no reader here, and asking them would spawn
        // four gits per member to discard four answers.
        members
            .iter()
            .map(|member| look(member, &plane, Weigh::Nothing, context))
            .collect::<Result<Vec<Checked>, EngineError>>()?
    } else {
        check_every(&members, &plane, &waivers, Teardown::Destroy, context)?
    };

    // Skipped entirely on a latched plane: no `post_worktree_create` ever ran
    // there, so firing its counterpart would hand a script a worktree it never
    // set up — and a blocking one would break the refusal-free guarantee that
    // makes `destroy` the only way out of a latch.
    let scripts = tear_down(&members, &plane, incomplete, request.run_scripts, context)?;

    let rows = take_out(&checked, incomplete, context);
    let interrupted = context.interrupt.is_raised();

    if rows.iter().any(PerMember::is_failure) {
        return Err(EngineError::RemovalFailed {
            plane: plane.id().to_owned(),
            operation: Teardown::Destroy,
            members: rows,
        });
    }

    // Only once every worktree is gone, and in this order: the file after the
    // worktrees, the directory after the file.
    if !interrupted {
        plane.remove_plane_file()?;
        plane.remove_directory()?;
    }

    Ok(PlaneDestroyed {
        id: plane.id().to_owned(),
        directory: plane.path().to_path_buf(),
        members: rows,
        incomplete,
        interrupted,
        scripts,
    })
}

/// The `pre_worktree_remove` pass, before anything is removed.
///
/// Returns `Err(ScriptBlocked)` the moment one exits non-zero, with nothing
/// touched. A block partway through leaves earlier members' teardown already
/// done with nothing removed — the accepted cost of letting one script both
/// veto and tear down, and the user's script and the user's unwind.
fn tear_down(
    members: &[Member],
    plane: &OpenPlane,
    latched: bool,
    run_scripts: bool,
    context: &TeardownContext<'_>,
) -> Result<Vec<ScriptOutcome>, EngineError> {
    if latched || !run_scripts {
        return Ok(Vec::new());
    }

    let outcomes = scripts::at_point(
        ScriptPoint::PreWorktreeRemove,
        members,
        InPlane {
            id: plane.id(),
            directory: plane.path(),
        },
        Stop::AtTheFirstFailure,
        &context.scripting(),
    )?;

    if outcomes.iter().any(|outcome| !outcome.succeeded()) {
        return Err(EngineError::ScriptBlocked { outcomes });
    }

    Ok(outcomes)
}

/// Takes the members named out of a plane, leaving the rest of it alone.
pub fn plane_remove(
    request: &PlaneRemoveRequest,
    context: &TeardownContext<'_>,
) -> Result<PlaneRemoved, EngineError> {
    let plane = OpenPlane::at(read::resolve(&request.plane, context.directories)?);
    let _lock = plane.lock(context.on_lock_wait)?;

    // Whittling a latched plane down one member at a time leaves a latched
    // plane, still headed for deletion, with a shorter membership list.
    if plane.is_latched() {
        return Err(EngineError::PlaneIncomplete {
            id: plane.id().to_owned(),
        });
    }

    let file = plane
        .read_plane_file()?
        .ok_or_else(|| EngineError::PlaneIncomplete {
            id: plane.id().to_owned(),
        })?;
    let named = named_members(&request.members, &file, &plane, context)?;

    let waivers = Waivers::new(request.waive.clone());
    let checked = check_every(&named, &plane, &waivers, Teardown::Remove, context)?;
    let scripts = tear_down(&named, &plane, false, request.run_scripts, context)?;

    let rows = take_out(&checked, false, context);

    if rows.iter().any(PerMember::is_failure) {
        return Err(EngineError::RemovalFailed {
            plane: plane.id().to_owned(),
            operation: Teardown::Remove,
            members: rows,
        });
    }

    // **Last**, so a crash between the removals and here leaves a *listed*
    // member with no worktree — visible, and converging on a re-run — rather
    // than an unlisted worktree nothing can see.
    rewrite_without(&plane, gone(&rows, &checked))?;

    Ok(PlaneRemoved {
        id: plane.id().to_owned(),
        directory: plane.path().to_path_buf(),
        members: rows,
        interrupted: context.interrupt.is_raised(),
        scripts,
    })
}

/// One member, looked at but not yet touched.
struct Checked {
    member: Member,
    /// Where its worktree is, absolutely.
    at: PathBuf,
    /// The repo it came from, where that is still there.
    source: Option<PathBuf>,
    /// Whether bitplane owns that repo, and so whether the branch the worktree
    /// is on is the plane's to delete.
    owned: bool,
    /// What git said about it.
    seen: Inspection,
    /// The reasons raised for it and waived — what the response echoes, so a
    /// forced destruction is visible in a transcript.
    waived: Vec<Reason>,
}

/// What asking git about one member produced.
///
/// **"git would not answer" is a third answer, not an error.** Letting it
/// propagate as a whole-operation failure would make one member with a broken
/// administrative link a plane nothing could take apart — no rows, and no
/// waiver that helps — which is the opposite of converging.
enum Inspection {
    /// Nothing registered and nothing on disk: an earlier run already took this
    /// one out.
    Gone,
    /// Git answered.
    Seen {
        /// Whether git records a worktree of this source at that path at all.
        /// A directory that is there but unregistered is just a directory:
        /// `git worktree remove` has nothing to unregister and refuses, so it
        /// has to go without one.
        registered: bool,
        /// The branch it is on, where it is on one.
        branch: Option<String>,
        /// Git's own lock on it, where someone took one.
        locked: Option<WorktreeLock>,
        /// What its contents are, absent where the directory git records is no
        /// longer there and so holds nothing to lose.
        state: Option<WorkingTree>,
        /// Whether its tip is on the forge, absent for the same reason.
        contained: Option<bool>,
    },
    /// Git could not be asked at all, and why.
    Unreachable(String),
}

/// Looks at every member and collects **every** reason before anything is
/// touched.
///
/// Returns `Err` the moment any reason survives the waivers — but only after
/// the whole pass has run, so the error names all of them and one re-run
/// finishes the job.
fn check_every(
    members: &[Member],
    plane: &OpenPlane,
    waivers: &Waivers,
    operation: Teardown,
    context: &TeardownContext<'_>,
) -> Result<Vec<Checked>, EngineError> {
    let mut checked: Vec<Checked> = Vec::new();
    let mut refusals: Vec<Refusal> = Vec::new();

    for member in members {
        let mut one = look(member, plane, Weigh::TheContents, context)?;
        let raised = reasons(&one);

        one.waived = raised
            .iter()
            .filter(|refusal| waivers.covers(refusal.reason))
            .map(|refusal| refusal.reason)
            .collect();

        checked.push(one);
        refusals.extend(raised);
    }

    let surviving = waivers.unwaived(&refusals);
    if surviving.is_empty() {
        return Ok(checked);
    }

    let blocking = members
        .iter()
        .filter(|member| {
            surviving
                .iter()
                .any(|refusal| refusal.member == member.source)
        })
        .count();

    Err(EngineError::Refused {
        plane: plane.id().to_owned(),
        operation,
        blocking,
        considered: members.len(),
        refusals: surviving.into_iter().cloned().collect(),
    })
}

/// How much of a member is worth asking about.
///
/// A latched plane runs no refusal checks, so the contents answers have no
/// reader there — and asking anyway would spawn four gits per member to throw
/// four answers away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Weigh {
    /// Where the worktree is and whether anything of it survives.
    Nothing,
    /// That, plus everything a refusal is made of.
    TheContents,
}

/// Everything git has to say about one member. No judgement — this half runs on
/// a latched plane too, where the answers are gathered but never weighed.
fn look(
    member: &Member,
    plane: &OpenPlane,
    weigh: Weigh,
    context: &TeardownContext<'_>,
) -> Result<Checked, EngineError> {
    let at = member.path.under(plane.path());
    let source = plan::source_of(&member.source, context.directories);

    let seen = match source.found() {
        Some(repo) => match inspect(repo, &at, weigh, source.owned, context) {
            Ok(seen) => seen,
            // Only git refusing. A git that could not be spawned at all keeps
            // its own typed failure, because that is not this member's fault
            // and no waiver should get past it.
            Err(EngineError::GitFailed { message }) => Inspection::Unreachable(message),
            Err(other) => return Err(other),
        },
        None => Inspection::Unreachable(format!(
            "the source repo is missing from {}",
            source.at.display()
        )),
    };

    Ok(Checked {
        member: member.clone(),
        owned: source.owned,
        source: source.found().map(Path::to_path_buf),
        at,
        seen,
        waived: Vec::new(),
    })
}

fn inspect(
    source: &Path,
    at: &Path,
    weigh: Weigh,
    owned: bool,
    context: &TeardownContext<'_>,
) -> Result<Inspection, EngineError> {
    let entry = context.git.worktree_at(source, at)?;

    if entry.is_none() && !at.exists() {
        return Ok(Inspection::Gone);
    }

    let branch = entry.as_ref().and_then(|entry| entry.branch.clone());

    if weigh != Weigh::TheContents {
        return Ok(Inspection::Seen {
            registered: entry.is_some(),
            branch,
            locked: entry.and_then(|entry| entry.locked),
            state: None,
            contained: None,
        });
    }

    // A directory git still records but that is no longer there holds no
    // *contents* to lose, so those questions have nothing to ask.
    let state = match at.is_dir() {
        true => Some(context.git.working_tree(at)?),
        false => None,
    };

    // The tip whose commits this teardown would put out of reach. From the
    // worktree while it is there — asked of the tip rather than of the branch
    // name, so a detached worktree is asked the same question — and otherwise
    // from the branch itself, but **only where that branch is about to be
    // deleted with it**. A directory the user removed by hand loses nothing
    // when bitplane owns no ref in the repo, and refusing there would be a
    // refusal over work that is not going anywhere.
    let contained = match (at.is_dir(), owned, &branch) {
        // No fetch, here or anywhere on this path: a stale tracking ref can
        // only make the answer more conservative (ADR-0006).
        (true, _, _) => Some(match context.git.head_commit(at)? {
            Some(tip) => context.git.contained_in_origin(source, &tip)?,
            None => false,
        }),
        (false, true, Some(branch)) => match context.git.branch_tip(source, branch)? {
            Some(tip) => Some(context.git.contained_in_origin(source, &tip)?),
            None => None,
        },
        (false, _, _) => None,
    };

    Ok(Inspection::Seen {
        registered: entry.is_some(),
        branch,
        locked: entry.and_then(|entry| entry.locked),
        state,
        contained,
    })
}

/// Everything one member would lose, in bitplane's own words.
fn reasons(one: &Checked) -> Vec<Refusal> {
    let mut refusals = Vec::new();
    let raise = |refusals: &mut Vec<Refusal>, reason, detail| {
        refusals.push(Refusal::new(one.member.source.clone(), reason, detail));
    };

    let (branch, locked, state, contained) = match &one.seen {
        Inspection::Gone => return refusals,
        // Different in kind from the four below. They mean *I know there is
        // work here and I accept losing it*; this one means *I cannot check at
        // all*, so folding it into `uncommitted` would let a waiver granted for
        // a diff the user looked at authorise deleting a directory nobody
        // looked at.
        Inspection::Unreachable(why) => {
            raise(&mut refusals, Reason::SourceRepoMissing, why.clone());
            return refusals;
        }
        Inspection::Seen {
            branch,
            locked,
            state,
            contained,
            ..
        } => (branch, locked, state, contained),
    };

    if let Some(lock) = locked {
        raise(
            &mut refusals,
            Reason::LockedWorktree,
            match &lock.reason {
                Some(reason) => format!("the worktree is locked: {reason}"),
                None => "the worktree is locked".to_owned(),
            },
        );
    }

    let subject = branch.clone().unwrap_or_else(|| "the worktree".to_owned());

    if let Some(state) = state {
        if state.uncommitted {
            raise(
                &mut refusals,
                Reason::Uncommitted,
                format!("{subject} has uncommitted changes"),
            );
        }
        if state.untracked {
            raise(
                &mut refusals,
                Reason::Untracked,
                format!("{subject} has untracked files"),
            );
        }
    }

    if contained == &Some(false) {
        raise(
            &mut refusals,
            Reason::Unpushed,
            format!("{subject} has commits that are not on origin"),
        );
    }

    refusals
}

/// Removes each member's worktree, in `plane.toml` order.
///
/// Sequential, unlike `create`'s fan-out: the removals are cheap, and doing
/// them in the order the file lists them is the order every row prints.
fn take_out(
    checked: &[Checked],
    latched: bool,
    context: &TeardownContext<'_>,
) -> Vec<PerMember<RemovedMember>> {
    let mut rows = Vec::with_capacity(checked.len());

    for one in checked {
        let outcome = if context.interrupt.is_raised() {
            // Converging, so an unscheduled member is simply work a re-run
            // still has to do.
            Outcome::Skipped(SkipReason::Interrupted)
        } else if matches!(one.seen, Inspection::Gone) {
            Outcome::AlreadyDone
        } else {
            match remove_worktree(one, latched, context).and_then(|()| delete_branch(one, context))
            {
                Ok(disposition) => Outcome::Ok(RemovedMember {
                    branch: one.branch(),
                    path: one.member.path.clone(),
                    disposition,
                    waived: one.waived.clone(),
                }),
                Err(error) => Outcome::Failed(error),
            }
        };

        rows.push(PerMember::new(one.member.source.clone(), outcome));
    }

    rows
}

/// One `git worktree remove`, pushing exactly as hard as the waivers allow.
///
/// A member whose source repo is gone has no git to ask, so the directory is
/// removed outright — which is exactly what `--waive source_repo_missing`
/// bought.
fn remove_worktree(
    one: &Checked,
    latched: bool,
    context: &TeardownContext<'_>,
) -> Result<(), EngineError> {
    // Nothing to ask and nothing that would answer, so the directory goes
    // directly and the source repo — if there still is one — is pruned, so no
    // administrative entry survives holding a branch nothing can see.
    if let Inspection::Unreachable(_) = one.seen {
        remove_directory(&one.at)?;

        let Some(source) = &one.source else {
            return Ok(());
        };
        // Prune skips a **locked** worktree, so a successful prune is not the
        // same as a cleared entry and the entry has to be confirmed gone. Where
        // git will not answer either question there is nothing more to be done,
        // and that is exactly the "I cannot check at all" the waiver bought.
        let cleared = context.git.prune_worktrees(source).is_ok();
        if cleared && matches!(context.git.worktree_at(source, &one.at), Ok(Some(_))) {
            return Err(EngineError::GitFailed {
                message: format!(
                    "git still records a worktree at {}; unlock it and try again",
                    one.at.display()
                ),
            });
        }

        return Ok(());
    }

    let Some(source) = &one.source else {
        return remove_directory(&one.at);
    };

    // Git has no record of a worktree there, so there is nothing to unregister
    // and `git worktree remove` would only refuse. The refusal pass has already
    // looked at what is in the directory, so removing it is the whole job.
    if matches!(
        one.seen,
        Inspection::Seen {
            registered: false,
            ..
        }
    ) {
        return remove_directory(&one.at);
    }

    if let Err(refused) =
        context
            .git
            .remove_worktree(source, &one.at, insistence(&one.waived, latched))
    {
        // An entry whose directory something already deleted is prune's
        // business and nothing else's — and prune skips a locked worktree, so
        // the entry has to be confirmed gone rather than assumed.
        if !context
            .git
            .worktree_at(source, &one.at)?
            .is_some_and(|entry| entry.prunable)
        {
            return Err(refused);
        }
        context.git.prune_worktrees(source)?;
        if context.git.worktree_at(source, &one.at)?.is_some() {
            return Err(refused);
        }
    }

    // Git removes the directory it recorded; anything the user left beside it
    // under the same path is still the plane's to clear.
    remove_directory(&one.at)
}

/// The branch the worktree was **actually on**, deleted where bitplane owns the
/// repo it lives in.
///
/// Actually on, not created on: nothing records the branch a worktree was made
/// with, so a user who switched branches gets the one they are standing on
/// deleted and the one they left behind survives with no record of it — clean
/// but not self-pruning, which is what `doctor` reports and never acts on.
///
/// Ordered after the removal because `git branch -D` refuses a branch a worktree
/// still holds; and forced, because `-d`'s merge check compares against the
/// branch's upstream, which for a plane branch is `origin/main` — the exact
/// wrong question, and one the `unpushed` refusal has already asked properly.
///
/// A branch that is **already gone** is the state this was asked to reach, so
/// it is reported as deleted rather than as a failure — which is what keeps a
/// concurrent or half-finished run converging. Anything else git refuses is a
/// per-member failure: the worktree is gone by then, so the plane keeps the
/// member and the run says so, and a re-run can only report the worktree
/// `already gone` — nothing records which branch it was on, so the branch is
/// left for `doctor`, which reports branches with no worktree and no plane.
fn delete_branch(
    one: &Checked,
    context: &TeardownContext<'_>,
) -> Result<BranchDisposition, EngineError> {
    let Some(branch) = one.branch() else {
        return Ok(BranchDisposition::None);
    };
    if !one.owned {
        return Ok(BranchDisposition::Kept);
    }
    let Some(source) = &one.source else {
        return Ok(BranchDisposition::None);
    };

    if !context.git.branch_exists(source, &branch)? {
        return Ok(BranchDisposition::Deleted);
    }

    context.git.delete_branch(source, &branch)?;

    Ok(BranchDisposition::Deleted)
}

/// How hard git may push, decided by what the user actually waived — or by the
/// latch, which says the same thing about every member of a plane at once.
///
/// `Polite` unless something licensed more. The second `--force` is for a lock,
/// which git wants the flag twice for and which nothing less removes.
///
/// The latched arm is **not** ticket 06's abort-path force borrowed: it is the
/// same justification reached independently. A latched plane was claimed and
/// never finished, so by construction nothing in it was ever the user's — and
/// since `destroy` is the only way out of a latch, a plane that could refuse
/// there would be a plane nothing could clear.
fn insistence(waived: &[Reason], latched: bool) -> Insistence {
    if latched || waived.contains(&Reason::LockedWorktree) {
        Insistence::ForcedThroughLock
    } else if waived.is_empty() {
        Insistence::Polite
    } else {
        Insistence::Forced
    }
}

fn remove_directory(at: &Path) -> Result<(), EngineError> {
    match std::fs::remove_dir_all(at) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(EngineError::io(at, err)),
    }
}

/// The worktree paths this run actually took out, so the rewrite drops exactly
/// those and leaves a member a re-run still has to reach.
fn gone(rows: &[PerMember<RemovedMember>], checked: &[Checked]) -> Vec<WorktreePath> {
    rows.iter()
        .zip(checked)
        .filter(|(row, _)| matches!(row.outcome, Outcome::Ok(_) | Outcome::AlreadyDone))
        .map(|(_, one)| one.member.path.clone())
        .collect()
}

/// Rewrites `plane.toml` without the members named, preserving everything else
/// the file holds — comments a user wrote included.
fn rewrite_without(plane: &OpenPlane, removed: Vec<WorktreePath>) -> Result<(), EngineError> {
    let path = plane.plane_file_path();
    let text = std::fs::read_to_string(&path).map_err(|err| EngineError::io(&path, err))?;

    plane.write_plane_file_text(&plane_file::without(&path, &text, &removed)?)
}

/// The members `bp rm` was asked about, in the order they were named.
fn named_members(
    specs: &[String],
    file: &PlaneFile,
    plane: &OpenPlane,
    context: &TeardownContext<'_>,
) -> Result<Vec<Member>, EngineError> {
    let mut named: Vec<Member> = Vec::new();

    for spec in specs {
        let reference = resolve(spec, context)?;

        let member = file
            .members
            .iter()
            .find(|member| member.source == reference)
            .ok_or_else(|| EngineError::ProjectNotInPlane {
                member: reference.to_string(),
                plane: plane.id().to_owned(),
            })?;

        if named.iter().any(|earlier| earlier.source == member.source) {
            return Err(EngineError::DuplicateMember {
                member: reference.to_string(),
                plane: None,
            });
        }

        named.push(member.clone());
    }

    Ok(named)
}

/// One member as the user wrote it. No branch suffix is read: a member is
/// already on a branch, and `rm` does not take one.
fn resolve(spec: &str, context: &TeardownContext<'_>) -> Result<MemberRef, EngineError> {
    Ok(match MemberRef::parse(spec)? {
        project @ MemberRef::Project(_) => project,
        // `plane.toml` holds canonical, symlink-followed paths, so a member
        // reached two ways has to arrive here spelled the one way the file
        // spells it — including a member whose repo has been **deleted**, which
        // is the one `--waive source_repo_missing` exists for and which plain
        // `canonicalize` cannot resolve at all.
        MemberRef::Repo(path) => {
            MemberRef::Repo(canonical_path(&plan::expand_home(&path, context.home)))
        }
    })
}

impl Checked {
    /// The branch the worktree was on, where git could say.
    fn branch(&self) -> Option<String> {
        match &self.seen {
            Inspection::Seen { branch, .. } => branch.clone(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_is_pushed_no_harder_than_the_waivers_allow() {
        assert_eq!(insistence(&[], false), Insistence::Polite);
        assert_eq!(
            insistence(&[Reason::Uncommitted], false),
            Insistence::Forced
        );
        assert_eq!(
            insistence(&[Reason::LockedWorktree], false),
            Insistence::ForcedThroughLock,
            "git wants --force twice for a worktree it has locked"
        );
    }

    #[test]
    fn a_latched_plane_is_forced_through_without_a_waiver_being_granted() {
        // Nothing in it was ever the user's, and `destroy` is the only way out
        // of a latch — so one that could refuse would be one nothing clears.
        assert_eq!(insistence(&[], true), Insistence::ForcedThroughLock);
    }
}
