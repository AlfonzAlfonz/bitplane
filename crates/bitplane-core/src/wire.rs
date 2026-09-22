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

use crate::engine::Engine;
use crate::error::EngineError;
use crate::head::Head;
use crate::health::{HealthCheck, PlaneHealth};
use crate::member::{MemberRef, WorktreePath};
use crate::outcome::PerMember;
use crate::plane_id::PlaneId;

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
    /// Every plane on this host.
    PlaneList(PlaneListRequest),
    /// One plane, in detail.
    PlaneShow(PlaneShowRequest),
    /// What git says about every member of one plane.
    PlaneStatus(PlaneStatusRequest),
}

/// One answer. Never a scalar count: a fan-out answers with a vector of keyed
/// per-item outcomes, so a progress stream is later that vector arriving early.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Response {
    PlaneCreate(PlaneCreated),
    PlaneList(PlaneList),
    PlaneShow(PlaneView),
    PlaneStatus(PlaneStatus),
}

impl Response {
    /// Whether this answer reported a finding, and therefore whether the
    /// command exits `3`.
    ///
    /// Drift is a state bitplane **reports**, not an error: the result is on
    /// stdout either way, and only the exit code says to look at it (ADR-0003).
    pub fn has_findings(&self) -> bool {
        match self {
            Response::PlaneCreate(_) => false,
            Response::PlaneList(list) => {
                list.planes.iter().any(|plane| plane.health.has_findings())
            }
            Response::PlaneShow(plane) => plane.health.has_findings(),
            Response::PlaneStatus(status) => status.health.has_findings(),
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

/// Runs one request against one engine.
pub fn dispatch<E: Engine + ?Sized>(engine: &E, request: Request) -> Result<Response, EngineError> {
    match request {
        Request::PlaneCreate(request) => engine.plane_create(request).map(Response::PlaneCreate),
        Request::PlaneList(request) => engine.plane_list(request).map(Response::PlaneList),
        Request::PlaneShow(request) => engine.plane_show(request).map(Response::PlaneShow),
        Request::PlaneStatus(request) => engine.plane_status(request).map(Response::PlaneStatus),
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
        });

        let json = serde_json::to_string(&request).unwrap();

        assert!(json.contains(r#""action":"plane_create""#), "got {json}");
        assert_eq!(serde_json::from_str::<Request>(&json).unwrap(), request);
    }

    #[test]
    fn the_default_intent_is_to_resolve() {
        assert_eq!(BranchIntent::default(), BranchIntent::Resolve);
    }
}
