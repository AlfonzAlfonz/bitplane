//! What a fan-out answers with.
//!
//! **Never a scalar count.** Every long operation returns a vector of keyed
//! per-member outcomes (ADR-0003), because those rows are the repair
//! instruction when some of them did not work — and because a progress stream
//! is later that same vector arriving early, so the shape is the streaming
//! hedge as well as the result.
//!
//! Keyed by [`MemberRef`] rather than by worktree path: the path is the field
//! `bp repair` rewrites when a worktree moves, so keying on it would label the
//! same member differently before and after a move, destroying the cross-run
//! correlation the keyed vector exists to provide (ADR-0008).

use serde::{Deserialize, Serialize};

use crate::error::EngineError;
use crate::member::{MemberRef, ProjectName};

/// One member's row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerMember<T> {
    pub member: MemberRef,
    /// Flattened, so a row is one flat object on the wire: the key beside the
    /// outcome rather than nested under a field called the same word.
    #[serde(flatten)]
    pub outcome: Outcome<T>,
}

/// One project's row.
///
/// The same shape as [`PerMember`] under a different key, because the two key
/// different things and ADR-0003 says so: [`MemberRef`] keys every plane
/// fan-out, and [`ProjectName`] keys the `project_*` actions. Spelled twice
/// rather than made generic, because the key's *name* is part of the wire
/// format — a reader of a row should not have to know which fan-out it came
/// from to read its subject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerProject<T> {
    pub project: ProjectName,
    #[serde(flatten)]
    pub outcome: Outcome<T>,
}

/// What happened to one member.
///
/// Adjacently tagged, so the tag is readable without knowing `T` and every
/// arm's payload — a struct, a bare reason, a typed failure — serialises the
/// same way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", content = "detail", rename_all = "snake_case")]
pub enum Outcome<T> {
    /// It was done by this run.
    Ok(T),
    /// It was already in the state asked for.
    AlreadyDone,
    /// It was not attempted.
    Skipped(SkipReason),
    /// It was attempted and did not work. A per-member failure is **data, not
    /// an error**: once the fan-out has begun, the envelope is no longer the
    /// place for it.
    Failed(EngineError),
}

/// Why a member was not attempted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SkipReason {
    /// Scheduling stopped because an earlier member failed. Applies to the
    /// `create`/`add` family, where building worktrees that are about to be
    /// unwound is pure waste.
    AbortedAfterEarlierFailure,
    /// Scheduling stopped because the run was interrupted.
    Interrupted,
    /// An **adopted** project, on a fetch. Its source repo is the user's
    /// checkout, so there is no forge to bring it up to date with. Naming one
    /// is not an error; it is a row, and never a denominator.
    NothingToFetch,
}

impl<T> PerMember<T> {
    pub fn new(member: MemberRef, outcome: Outcome<T>) -> PerMember<T> {
        PerMember { member, outcome }
    }

    pub fn ok(member: MemberRef, value: T) -> PerMember<T> {
        PerMember::new(member, Outcome::Ok(value))
    }

    pub fn skipped(member: MemberRef, reason: SkipReason) -> PerMember<T> {
        PerMember::new(member, Outcome::Skipped(reason))
    }

    pub fn failed(member: MemberRef, error: EngineError) -> PerMember<T> {
        PerMember::new(member, Outcome::Failed(error))
    }

    /// Whether this row is the one that stopped the run.
    pub fn is_failure(&self) -> bool {
        matches!(self.outcome, Outcome::Failed(_))
    }

    /// What this row produced, where it produced anything.
    pub fn value(&self) -> Option<&T> {
        match &self.outcome {
            Outcome::Ok(value) => Some(value),
            _ => None,
        }
    }
}

impl<T> PerProject<T> {
    pub fn new(project: ProjectName, outcome: Outcome<T>) -> PerProject<T> {
        PerProject { project, outcome }
    }

    pub fn ok(project: ProjectName, value: T) -> PerProject<T> {
        PerProject::new(project, Outcome::Ok(value))
    }

    pub fn skipped(project: ProjectName, reason: SkipReason) -> PerProject<T> {
        PerProject::new(project, Outcome::Skipped(reason))
    }

    pub fn failed(project: ProjectName, error: EngineError) -> PerProject<T> {
        PerProject::new(project, Outcome::Failed(error))
    }

    /// Whether this row is one that did not work.
    pub fn is_failure(&self) -> bool {
        matches!(self.outcome, Outcome::Failed(_))
    }

    /// What this row produced, where it produced anything.
    pub fn value(&self) -> Option<&T> {
        match &self.outcome {
            Outcome::Ok(value) => Some(value),
            _ => None,
        }
    }
}

impl SkipReason {
    /// The clause a fan-out row prints after `skipped:`.
    pub fn reason(self) -> &'static str {
        match self {
            SkipReason::AbortedAfterEarlierFailure => "aborted after an earlier failure",
            SkipReason::Interrupted => "interrupted",
            SkipReason::NothingToFetch => "nothing to fetch (adopted)",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_row_round_trips_over_the_wire_carrying_its_key() {
        let row: PerMember<String> = PerMember::ok(
            MemberRef::Repo(PathBuf::from("/repos/api")),
            "feat-login".to_owned(),
        );

        let json = serde_json::to_string(&row).unwrap();
        assert_eq!(
            serde_json::from_str::<PerMember<String>>(&json).unwrap(),
            row
        );
        assert!(json.contains("/repos/api"), "got {json}");
    }

    #[test]
    fn a_failed_row_carries_the_typed_failure_as_data() {
        let row: PerMember<String> = PerMember::failed(
            MemberRef::Repo(PathBuf::from("/repos/api")),
            EngineError::GitMissing,
        );

        assert!(row.is_failure());
        assert_eq!(row.value(), None);

        let json: serde_json::Value = serde_json::to_value(&row).unwrap();
        assert_eq!(json["outcome"], "failed");
        assert_eq!(json["detail"]["error"], "git_missing");
    }

    #[test]
    fn every_skip_reason_says_why_in_the_words_a_row_prints() {
        for reason in [
            SkipReason::AbortedAfterEarlierFailure,
            SkipReason::Interrupted,
            SkipReason::NothingToFetch,
        ] {
            assert!(!reason.reason().is_empty(), "for {reason:?}");
        }
    }

    #[test]
    fn a_project_row_carries_its_name_as_the_key_the_wire_uses() {
        let row: PerProject<u32> = PerProject::ok(ProjectName::parse("codestyle").unwrap(), 3);

        let json = serde_json::to_string(&row).unwrap();

        assert!(json.contains(r#""project":"codestyle""#), "got {json}");
        assert_eq!(serde_json::from_str::<PerProject<u32>>(&json).unwrap(), row);
    }
}
