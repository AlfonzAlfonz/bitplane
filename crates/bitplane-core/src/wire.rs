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
use crate::member::WorktreePath;
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
}

/// One answer. Never a scalar count: a fan-out answers with a vector of keyed
/// per-item outcomes, so a progress stream is later that vector arriving early.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Response {
    PlaneCreate(PlaneCreated),
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

/// Runs one request against one engine.
pub fn dispatch<E: Engine + ?Sized>(engine: &E, request: Request) -> Result<Response, EngineError> {
    match request {
        Request::PlaneCreate(request) => engine.plane_create(request).map(Response::PlaneCreate),
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
