//! What crosses a transport.
//!
//! The traits are the ergonomic face; this pair plus [`dispatch`] is what a
//! remote surface speaks (ADR-0003). `dispatch` is a `match`, so an action put
//! on the wire and forgotten in the dispatch fails to compile.
//!
//! One request is one user intent — coarse granularity, so a remote transport
//! is not a latency disaster — and a response is a single value, with streamed
//! progress deferred.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::engine::{Engine, Reader};
use crate::error::{EngineError, Problem};
use crate::head::Head;
use crate::health::{HealthCheck, PlaneHealth};
use crate::member::{MemberRef, PROJECT_SIGIL, ProjectName, WorktreePath};
use crate::outcome::{Outcome, PerMember, PerProject};
use crate::plane_id::PlaneId;
use crate::project_file::ProjectSource;
use crate::refusal::Reason;

/// One user intent.
///
/// Deliberately **not** `#[non_exhaustive]`: every surface is supposed to match
/// it exhaustively, so that an action added here and forgotten at a rendering
/// fails to compile — the same property [`dispatch`] has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Request {
    /// Make a plane, and a worktree of every member named.
    PlaneCreate(PlaneCreateRequest),
    /// Put members into a plane that already exists.
    PlaneAdd(PlaneAddRequest),
    /// Every plane on this host.
    PlaneList(PlaneListRequest),
    /// One plane, in detail.
    PlaneShow(PlaneShowRequest),
    /// What git says about every member of one plane.
    PlaneStatus(PlaneStatusRequest),
    /// Register a project from a URL, building the source repo bitplane owns.
    ProjectAdd(ProjectAddRequest),
    /// Every project registered on this host.
    ProjectList,
    /// Bring owned projects' source repos up to date with their forges.
    ProjectFetch(ProjectFetchRequest),
    /// Take a whole plane apart.
    PlaneDestroy(PlaneDestroyRequest),
    /// Take named members out of a plane, leaving the rest alone.
    PlaneRemove(PlaneRemoveRequest),
    /// Run a project's declared scripts, by name, on demand.
    PlaneScripts(PlaneScriptsRequest),
}

/// One answer. Never a scalar count: a fan-out answers with a vector of keyed
/// per-item outcomes, so a progress stream is later that vector arriving early.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Response {
    PlaneCreate(PlaneCreated),
    PlaneAdd(PlaneAdded),
    PlaneList(PlaneList),
    PlaneShow(PlaneView),
    PlaneStatus(PlaneStatus),
    ProjectAdd(ProjectAdded),
    ProjectList(ProjectListing),
    ProjectFetch(ProjectFetched),
    PlaneDestroy(PlaneDestroyed),
    PlaneRemove(PlaneRemoved),
    PlaneScripts(ScriptsRun),
}

impl Response {
    /// Whether this answer reported a finding, and therefore whether the
    /// command exits `3`.
    ///
    /// Drift is a state bitplane **reports**, not an error: the result is on
    /// stdout either way, and only the exit code says to look at it (ADR-0003).
    pub fn has_findings(&self) -> bool {
        match self {
            // Neither a teardown nor a build reports drift: both say what they
            // did in their rows, and a failure is an envelope rather than an
            // exit code to look up. A script run is the same.
            Response::PlaneCreate(_)
            | Response::PlaneAdd(_)
            | Response::PlaneDestroy(_)
            | Response::PlaneRemove(_)
            | Response::PlaneScripts(_) => false,
            Response::PlaneList(list) => {
                list.planes.iter().any(|plane| plane.health.has_findings())
            }
            Response::PlaneShow(plane) => plane.health.has_findings(),
            Response::PlaneStatus(status) => status.health.has_findings(),
            Response::ProjectAdd(_) | Response::ProjectFetch(_) => false,
            // The one finding that is not about a plane: a `project.toml` that
            // will not parse is a row the listing carried on past, and the
            // exit code is what says to look at it.
            Response::ProjectList(listing) => listing.has_unreadable(),
        }
    }
}

/// Make a plane.
///
/// Members arrive as the strings the user wrote, because telling
/// `~/projects/weird:name` from `~/projects/bitplane:feat-x` needs the
/// filesystem, and the engine is the side that has one — the same reason
/// ADR-0003 puts plane discovery there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneCreateRequest {
    /// One or more members, as written: `@name`, or a path, each optionally
    /// carrying `:branch`.
    pub members: Vec<String>,
    /// The branch for every member that does not carry its own. Applied at
    /// this moment and not remembered: a plane has no branch.
    pub branch: Option<String>,
    /// The plane id. `None` generates one.
    pub id: Option<String>,
    pub intent: BranchIntent,
    /// Whether to bring every **owned** project named up to date first, so the
    /// branch resolves against what the forge actually has.
    ///
    /// A field on the request rather than a behaviour inside [`BranchIntent`],
    /// so the intent stays a pure statement of intent and `Resolve` with
    /// `fetch: false` can be refused at request validation (ADR-0007).
    pub fetch: bool,
    /// Whether to run each member's `post_worktree_create` scripts once the
    /// plane is complete.
    pub run_scripts: bool,
}

/// Put members into a plane that already exists.
///
/// `add` cannot abort-and-remove the way `create` does — the plane holds other
/// members full of work — so a failure force-removes only the worktrees **it**
/// created and drops only **its own** entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneAddRequest {
    pub plane: PlaneRef,
    /// One or more members, as written: `@name`, or a path, each optionally
    /// carrying `:branch`.
    pub members: Vec<String>,
    /// The branch for every member that does not carry its own.
    ///
    /// **Mandatory, from here or from a suffix.** Nothing is derived from what
    /// the plane's existing members happen to be on: that would make `add`
    /// depend on unrelated work in another worktree.
    pub branch: Option<String>,
    pub intent: BranchIntent,
    pub fetch: bool,
    /// Whether to run each member's `post_worktree_create` scripts.
    pub run_scripts: bool,
}

/// Whether the branch asked for must exist, must not, or either.
///
/// An intent flag rather than a lookup round trip: the ambiguity costs one
/// field instead of a "does this branch exist?" call (ADR-0003).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BranchIntent {
    /// Check the branch out if it resolves, and create it otherwise.
    #[default]
    Resolve,
    /// The branch must already exist.
    RequireExisting,
    /// The branch must not already exist.
    RequireNew,
}

/// A plane that was made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneCreated {
    pub id: PlaneId,
    pub directory: PathBuf,
    /// One row per member, in the order they were named.
    pub members: Vec<PerMember<CreatedMember>>,
    /// Whether Ctrl-C stopped the run. The rows are then what was unwound and
    /// what was never attempted, and they are the repair instruction rather
    /// than a courtesy (ADR-0004).
    pub interrupted: bool,
    /// Whether the abort path could not take the whole plane back. Only an
    /// interrupted run can answer `Ok` with this set — a failed one says the
    /// same thing through `CreateAborted`'s own `remnant`, and a plane that was
    /// built has nothing to leave behind.
    pub remnant: bool,
    /// What each `post_worktree_create` script did, in the order they ran.
    /// Empty where no member declared one, or where `--no-scripts` was given.
    pub scripts: Vec<ScriptOutcome>,
}

/// Members that were put into a plane that already existed.
///
/// There is no `remnant` and no latch: `add` never writes the incomplete
/// marker, under any failure. The latch means *this plane was never completed,
/// nothing in it is yours*, and a plane full of real work flagged as free to
/// discard would be a trapdoor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneAdded {
    /// The **directory's name**, which is the plane's identity — a string for
    /// the same reason [`PlaneView`]'s is.
    pub id: String,
    pub directory: PathBuf,
    /// One row per member **named**, in the order they were named.
    pub members: Vec<PerMember<CreatedMember>>,
    pub interrupted: bool,
    /// Whether the unwind could not take this run's members back, so the plane
    /// is **not** as it was found and lists a member with no worktree. Only an
    /// interrupted run can answer `Ok` with this set — a failed one says the
    /// same thing through `AddAborted`'s own `remnant`.
    pub remnant: bool,
    /// What each `post_worktree_create` script did, in the order they ran.
    pub scripts: Vec<ScriptOutcome>,
}

/// One member's worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatedMember {
    /// The branch the worktree is on.
    pub branch: String,
    /// Whether that branch had to be cut. The difference between joining a
    /// colleague's work and starting your own.
    pub created_branch: bool,
    /// Where the worktree is, relative to the plane directory.
    pub path: WorktreePath,
    /// Whether the abort path took it back again.
    pub unwound: bool,
}

/// Which plane an action is about.
///
/// The path arm exists because a client cannot walk a *remote* filesystem up to
/// `plane.toml`: without it every in-plane command would be resolve-then-act,
/// two round trips, which is the precise failure mode coarse granularity exists
/// to prevent (ADR-0003). The walk therefore runs in the engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "plane", rename_all = "snake_case")]
pub enum PlaneRef {
    /// Named outright, with `-p`.
    Id { id: String },
    /// The innermost plane containing this path. **Resolves to a plane, never
    /// to a member** (ADR-0008).
    ContainingPath { path: PathBuf },
}

/// Every plane on this host.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneListRequest {
    pub health: HealthCheck,
}

/// One plane, in detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneShowRequest {
    pub plane: PlaneRef,
    pub health: HealthCheck,
}

/// What git says about every member of one plane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneStatusRequest {
    pub plane: PlaneRef,
}

/// The planes, in directory-name order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneList {
    pub planes: Vec<PlaneView>,
}

/// One plane, as a read describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneView {
    /// The **directory's name**, which is the plane's identity — held as a
    /// string rather than a [`PlaneId`] because a directory a user made by hand
    /// need not be one. `plane.toml`'s own `id` is a self-check, and a
    /// disagreement is reported as [`crate::Finding::IdMismatch`].
    pub id: String,
    pub directory: PathBuf,
    /// The plane directory's birth time, RFC 3339. Absent where the filesystem
    /// cannot supply one, and nothing gates on it (ADR-0008).
    pub created_at: Option<String>,
    /// In plane file order, which is the order every fan-out prints. Empty
    /// where the plane file could not be read.
    pub members: Vec<MemberView>,
    pub health: PlaneHealth,
}

/// One member, and the branch its worktree is on right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberView {
    pub member: MemberRef,
    pub path: WorktreePath,
    /// Read from the worktree's `HEAD` at the moment of the read, so it is true
    /// by construction: `plane.toml` records no branch, and nothing derives one.
    pub head: Option<Head>,
}

/// git's own answer about every member of one plane, rendered and stored
/// nowhere — which is what makes it unable to go stale (ADR-0006).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneStatus {
    pub id: String,
    pub directory: PathBuf,
    /// In plane file order.
    pub members: Vec<MemberStatus>,
    pub health: PlaneHealth,
}

/// One member's row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberStatus {
    pub member: MemberRef,
    pub path: WorktreePath,
    pub head: Option<Head>,
    pub work: MemberWork,
}

/// What git had to say about one worktree.
///
/// Deliberately **not** `#[non_exhaustive]`, for the same reason [`Response`] is
/// not: every surface renders it, so a state added here and forgotten at a
/// rendering has to fail to compile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "work", rename_all = "snake_case")]
pub enum MemberWork {
    /// git answered. **None of these three is drift**: a worktree's contents
    /// and its `HEAD` are the user's, and this is reported at exit `0`.
    Reported {
        /// Tracked files with changes, staged or not.
        modified: usize,
        /// Files git does not know about.
        untracked: usize,
        /// Commits on this branch contained in no `refs/remotes/origin/*`.
        /// Never `git branch -d`'s upstream comparison, which reports a branch
        /// already safe on the forge as unmerged (ADR-0006). No fetch is
        /// performed, so a stale tracking ref can only make this too large.
        ahead: usize,
    },
    /// There is nothing at the member's path to ask about.
    WorktreeMissing,
    /// The repository the worktree comes from is gone.
    SourceRepoMissing,
    /// git ran and did not succeed. Data on a row, not a failure of the read.
    Unreadable { message: String },
}

/// Register a project from a URL.
///
/// One request per intent, not one per command line: a default name colliding
/// and being re-run with `--name <suggestion>` is **two** requests, because the
/// caller supplied new information in between (ADR-0003).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAddRequest {
    /// Anything git can fetch from. bitplane speaks the git protocol and no
    /// forge API, and never reads, stores, prompts for or forwards a
    /// credential.
    pub url: String,
    /// The project's name. `None` takes the last segment of the URL's path.
    pub name: Option<String>,
}

/// Bring owned projects up to date.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectFetchRequest {
    /// Which projects, as `@name` or `name`. **Empty takes every registered
    /// project in turn.**
    pub projects: Vec<String>,
}

/// A project that was registered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAdded {
    pub name: ProjectName,
    pub source: ProjectSource,
    /// The project directory: `project.toml`, `repo.git`, and the `bin/` the
    /// user may create.
    pub directory: PathBuf,
    /// The base a new plane branch would be cut from. `None` where the forge
    /// named no default — not an error by itself, and only refused at the
    /// moment a new branch has to be cut (ADR-0005).
    pub default_branch: Option<String>,
}

/// Every project on this host, one row each.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectListing {
    pub projects: Vec<PerProject<ProjectSummary>>,
}

/// One project, as a listing sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub source: ProjectSource,
    /// Whether the source repo is on this host. One `stat`, never a stored
    /// field (ADR-0007).
    pub source_repo_present: bool,
}

/// What a fetch did, project by project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectFetched {
    pub projects: Vec<PerProject<Fetched>>,
    /// Whether Ctrl-C stopped the run. The remaining rows are then `skipped`.
    pub interrupted: bool,
}

/// One project's fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fetched {
    /// How many `refs/remotes/origin/*` this fetch added, moved or pruned.
    /// Zero is "up to date".
    pub updated: usize,
}

impl ProjectListing {
    /// Whether any project's file could not be read.
    ///
    /// A listing tolerates a file it cannot read — the row is the result and
    /// the scan carries on — so this is what turns it into drift rather than a
    /// failure (ADR-0003).
    pub fn has_unreadable(&self) -> bool {
        self.projects.iter().any(PerProject::is_failure)
    }
}

impl ProjectFetched {
    /// The failure a run with at least one failed fetch reports **alongside**
    /// its rows.
    ///
    /// The rows are the result and go to stdout regardless; this is what says
    /// the run did not fully succeed. `None` where every fetchable project was
    /// fetched.
    pub fn failure(&self) -> Option<EngineError> {
        let problems: Vec<Problem> = self
            .projects
            .iter()
            .filter_map(|row| match &row.outcome {
                Outcome::Failed(error) => Some(Problem::about(
                    format!("{PROJECT_SIGIL}{}", row.project),
                    error.to_string(),
                )),
                _ => None,
            })
            .collect();

        if problems.is_empty() {
            return None;
        }

        Some(EngineError::FetchFailed {
            failed: problems.len(),
            fetchable: self.fetchable(),
            problems,
        })
    }

    /// The projects there was something to fetch **for**. An adopted project is
    /// a row, never a denominator.
    fn fetchable(&self) -> usize {
        self.projects
            .iter()
            .filter(|row| {
                !matches!(
                    row.outcome,
                    Outcome::Skipped(crate::outcome::SkipReason::NothingToFetch)
                )
            })
            .count()
    }
}

/// Take a whole plane apart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneDestroyRequest {
    pub plane: PlaneRef,
    /// The reasons this invocation accepts losing work over. Granted per reason
    /// and per invocation; there is no blanket force flag.
    pub waive: Vec<Reason>,
    /// Whether to run each member's `pre_worktree_remove` scripts.
    ///
    /// **A `project.toml` can never make a plane undestroyable**, because this
    /// is on the request: a blocking pre-script is switched off here and the
    /// teardown proceeds. That is the whole answer, and it needed no new
    /// mechanism (ADR-0007).
    pub run_scripts: bool,
}

/// Take named members out of a plane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneRemoveRequest {
    pub plane: PlaneRef,
    /// The members, as written: `@name`, or a path. No branch suffix — a member
    /// is already on a branch.
    pub members: Vec<String>,
    pub waive: Vec<Reason>,
    /// Whether to run the named members' `pre_worktree_remove` scripts.
    ///
    /// ADR-0006 made `plane_remove` *identical* to `destroy`, so its absence
    /// here was an oversight rather than a decision (ADR-0007).
    pub run_scripts: bool,
}

/// Run a project's declared scripts, by name, on demand.
///
/// On [`Engine`] and not [`Reader`]: it runs arbitrary user commands and writes
/// logs, and *"a read never writes"* is supposed to be checkable by reading the
/// trait (ADR-0003).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneScriptsRequest {
    pub plane: PlaneRef,
    /// The script names, run in **request order**: unlike a lifecycle point,
    /// the caller named these, so that is the order they meant. Non-empty.
    pub names: Vec<String>,
    /// The projects whose scripts to run. **Mandatory** — nobody gets a
    /// six-repo script run by typing nothing — and each must be a member.
    pub projects: Vec<ProjectName>,
}

/// What a script run did, script by script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptsRun {
    pub id: String,
    pub directory: PathBuf,
    /// One row per script that ran, in the order they ran.
    pub scripts: Vec<ScriptOutcome>,
}

/// One script, and what became of it.
///
/// `finished_at` is **not optional**, so a script killed mid-run leaves no
/// record at all: "started but never finished" is unrepresentable, and Ctrl-C
/// is the escape (ADR-0004).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptOutcome {
    /// The script's own name, which every script has by construction.
    pub name: String,
    /// The project that declared it. Always present today; the `Option` is room
    /// to add plane-level scripts later without a contract change.
    pub project: Option<ProjectName>,
    /// Where the merged output was tee'd. bitplane never cleans these up;
    /// `destroy` removes them with the plane directory.
    pub log: PathBuf,
    /// RFC 3339, in UTC.
    pub finished_at: String,
    pub duration_ms: u64,
    /// Flattened, so a row is one flat object on the wire — the same shape
    /// [`PerMember`] gives an outcome.
    #[serde(flatten)]
    pub result: ScriptResult,
}

/// Whether a script worked, and what to say if it did not.
///
/// Deliberately **not** `#[non_exhaustive]`, for the same reason [`MemberWork`]
/// is not: every surface renders it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum ScriptResult {
    /// It exited zero.
    Ok,
    /// It exited non-zero, was ended by a signal, or never started.
    Failed {
        /// The process's exit status, absent where there was none to have.
        code: Option<i32>,
        /// The clause a message prints after the script's name: `exited 1`.
        detail: String,
    },
}

impl ScriptOutcome {
    /// Whether this script exited zero.
    pub fn succeeded(&self) -> bool {
        matches!(self.result, ScriptResult::Ok)
    }

    /// The clause a message prints after the script's name, where it did not
    /// work.
    pub fn failure(&self) -> Option<&str> {
        match &self.result {
            ScriptResult::Ok => None,
            ScriptResult::Failed { detail, .. } => Some(detail),
        }
    }

    /// The project, rendered as bitplane prints one.
    pub fn subject(&self) -> String {
        match &self.project {
            Some(project) => format!("{PROJECT_SIGIL}{project}"),
            None => String::new(),
        }
    }
}

/// A plane that was taken apart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneDestroyed {
    /// The **directory's name**, which is the plane's identity — a string for
    /// the same reason [`PlaneView`]'s is: a directory a user made by hand need
    /// not be a well-formed id, and refusing to name one would make it
    /// impossible to destroy.
    pub id: String,
    pub directory: PathBuf,
    /// One row per member, in `plane.toml` order.
    pub members: Vec<PerMember<RemovedMember>>,
    /// Whether `create` had never completed, so no refusal check ran at all —
    /// every reason is structurally impossible on a latched plane.
    pub incomplete: bool,
    /// Whether Ctrl-C stopped the run. `destroy` converges, so the rows are
    /// what is left to do rather than something to unwind.
    pub interrupted: bool,
    /// What each `pre_worktree_remove` script did. Empty on a latched plane,
    /// where the point is skipped entirely.
    pub scripts: Vec<ScriptOutcome>,
}

/// Members that were taken out of a plane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneRemoved {
    pub id: String,
    pub directory: PathBuf,
    /// One row per member **named**, in the order they were named.
    pub members: Vec<PerMember<RemovedMember>>,
    pub interrupted: bool,
    /// What each `pre_worktree_remove` script did.
    pub scripts: Vec<ScriptOutcome>,
}

/// One member's worktree, gone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovedMember {
    /// The branch the worktree was on, absent where it was detached or where
    /// the source repo could not be asked.
    pub branch: Option<String>,
    /// Where the worktree was, relative to the plane directory.
    pub path: WorktreePath,
    /// What became of that branch.
    pub disposition: BranchDisposition,
    /// The reasons that were raised for this member and waived, so a forced
    /// destruction is visible in a transcript.
    pub waived: Vec<Reason>,
}

/// What a teardown did with the branch a worktree was on.
///
/// Three answers rather than a boolean, because *bitplane deleted your branch*
/// and *bitplane left your branch alone because it owns nothing in that repo*
/// are different pieces of news and a user reading a row needs both (ADR-0006).
///
/// Deliberately **not** `#[non_exhaustive]`, for the same reason [`MemberWork`]
/// is not: every surface renders it, so a state added here and forgotten at a
/// rendering has to fail to compile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchDisposition {
    /// Deleted with the worktree. Only ever an **owned** project's branch: for
    /// a bitplane-owned source repo `refs/heads/*` holds exactly the branches
    /// plane members were created on, so the branch is the plane's to delete.
    Deleted,
    /// Left alone. bitplane never deletes a ref in an **adopted** project's
    /// checkout or an **ad-hoc member**'s repo — it cannot tell a branch it
    /// created from one the user has had for two years, and it owns nothing
    /// there.
    Kept,
    /// There was no branch to speak of: the worktree was detached, or git
    /// could not be asked.
    None,
}

/// Runs one request against one engine.
pub fn dispatch<E: Engine + ?Sized>(engine: &E, request: Request) -> Result<Response, EngineError> {
    match request {
        Request::PlaneCreate(request) => engine.plane_create(request).map(Response::PlaneCreate),
        Request::PlaneAdd(request) => engine.plane_add(request).map(Response::PlaneAdd),
        Request::PlaneList(request) => engine.plane_list(request).map(Response::PlaneList),
        Request::PlaneShow(request) => engine.plane_show(request).map(Response::PlaneShow),
        Request::PlaneStatus(request) => engine.plane_status(request).map(Response::PlaneStatus),
        Request::ProjectAdd(request) => engine.project_add(request).map(Response::ProjectAdd),
        Request::ProjectList => Reader::project_list(engine).map(Response::ProjectList),
        Request::ProjectFetch(request) => engine.project_fetch(request).map(Response::ProjectFetch),
        Request::PlaneDestroy(request) => engine.plane_destroy(request).map(Response::PlaneDestroy),
        Request::PlaneRemove(request) => engine.plane_remove(request).map(Response::PlaneRemove),
        Request::PlaneScripts(request) => engine.plane_scripts(request).map(Response::PlaneScripts),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_round_trips_over_the_wire() {
        let request = Request::PlaneCreate(PlaneCreateRequest {
            members: vec!["@api".to_owned(), "~/projects/bitplane:feat-x".to_owned()],
            branch: Some("feat-login".to_owned()),
            id: Some("auth-work".to_owned()),
            intent: BranchIntent::RequireNew,
            fetch: true,
            run_scripts: true,
        });

        let json = serde_json::to_string(&request).unwrap();

        assert!(json.contains(r#""action":"plane_create""#), "got {json}");
        assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), request);
    }

    #[test]
    fn a_destroy_request_round_trips_carrying_its_waivers() {
        let request = Request::PlaneDestroy(PlaneDestroyRequest {
            plane: PlaneRef::Id {
                id: "auth-work".to_owned(),
            },
            waive: vec![Reason::Uncommitted, Reason::Unpushed],
            run_scripts: true,
        });

        let json = serde_json::to_string(&request).unwrap();

        assert!(json.contains(r#""action":"plane_destroy""#), "got {json}");
        assert!(json.contains(r#""uncommitted""#), "got {json}");
        assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), request);
    }

    #[test]
    fn the_default_intent_is_to_resolve() {
        assert_eq!(BranchIntent::default(), BranchIntent::Resolve);
    }
}
