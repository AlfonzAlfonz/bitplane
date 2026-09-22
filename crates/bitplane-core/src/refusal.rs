//! What `destroy` and `remove` decline to do, and the five things a user may
//! say they accept losing.
//!
//! A **waiver** says *"I accept losing **this** work, which I am looking at"*.
//! A **refusal** is what is left when that sentence cannot honestly be said
//! (ADR-0006). The five reasons here are all of the first shape; the refusals
//! that are not — `project_in_use`, `plane_incomplete`, `branch_occupied` — have
//! no waiver at all and live in [`crate::EngineError`] instead.
//!
//! Waivers are granted **per reason and per invocation**. There is no blanket
//! `--force`, nothing is stored, and nothing is inherited by a later command —
//! so a waiver can never authorise a destruction the user was not looking at
//! when they granted it.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::EngineError;
use crate::member::MemberRef;

/// One reason bitplane declines to destroy something, and the waiver that
/// answers it.
///
/// The declaration order is the order a remedy lists them in, so a pasted
/// `--waive` line reads the same way every time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Reason {
    /// Tracked files differ from `HEAD`.
    Uncommitted,
    /// Files git does not track and no ignore rule covers.
    Untracked,
    /// The branch tip is not contained in any `refs/remotes/origin/*`.
    Unpushed,
    /// `git worktree lock`: someone said "do not remove this".
    LockedWorktree,
    /// The repo the worktree came from is not there, so nothing above can be
    /// checked at all.
    SourceRepoMissing,
}

/// One member's reason, with the sentence it prints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    pub member: MemberRef,
    pub reason: Reason,
    /// The sentence this refusal prints, in bitplane's own words. Built where
    /// the check ran, because only there is the branch known.
    pub detail: String,
}

/// The reasons this invocation accepts losing work over.
///
/// A set rather than a flag: waiving `uncommitted` says nothing about
/// `unpushed`, which is the entire point of there being no `--force`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Waivers {
    granted: Vec<Reason>,
}

impl Reason {
    /// Every reason, in the order a remedy lists them.
    pub const ALL: [Reason; 5] = [
        Reason::Uncommitted,
        Reason::Untracked,
        Reason::Unpushed,
        Reason::LockedWorktree,
        Reason::SourceRepoMissing,
    ];

    /// The tag, spelled exactly as `--waive` takes it and exactly as a refusal
    /// prints it — so the string you read is the string you type back.
    pub fn tag(self) -> &'static str {
        match self {
            Reason::Uncommitted => "uncommitted",
            Reason::Untracked => "untracked",
            Reason::Unpushed => "unpushed",
            Reason::LockedWorktree => "locked_worktree",
            Reason::SourceRepoMissing => "source_repo_missing",
        }
    }

    /// Reads a `--waive` value.
    pub fn parse(tag: &str) -> Result<Reason, EngineError> {
        Reason::ALL
            .into_iter()
            .find(|reason| reason.tag() == tag)
            .ok_or_else(|| EngineError::InvalidRequest {
                message: format!("{tag} is not a reason that can be waived"),
            })
    }
}

impl Refusal {
    pub fn new(member: MemberRef, reason: Reason, detail: impl Into<String>) -> Refusal {
        Refusal {
            member,
            reason,
            detail: detail.into(),
        }
    }
}

impl Waivers {
    /// Reads the `--waive` values as written.
    pub fn parse(tags: &[String]) -> Result<Waivers, EngineError> {
        tags.iter()
            .map(|tag| Reason::parse(tag))
            .collect::<Result<Vec<Reason>, EngineError>>()
            .map(Waivers::new)
    }

    pub fn new(granted: Vec<Reason>) -> Waivers {
        Waivers { granted }
    }

    /// Whether this reason was waived. Waiving one that was never raised is
    /// not an error — it is a line in a script that was right to be there.
    pub fn covers(&self, reason: Reason) -> bool {
        self.granted.contains(&reason)
    }

    /// The refusals that survive these waivers, in the order they were found.
    pub fn unwaived<'a>(&self, refusals: &'a [Refusal]) -> Vec<&'a Refusal> {
        refusals
            .iter()
            .filter(|refusal| !self.covers(refusal.reason))
            .collect()
    }
}

/// `--waive a --waive b`, for a remedy the user can paste.
///
/// Names **exactly the reasons that were raised**, in [`Reason::ALL`] order,
/// and never one that did not come up.
pub fn waive_flags(refusals: &[&Refusal]) -> String {
    Reason::ALL
        .into_iter()
        .filter(|reason| refusals.iter().any(|refusal| refusal.reason == *reason))
        .map(|reason| format!("--waive {}", reason.tag()))
        .collect::<Vec<String>>()
        .join(" ")
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.tag())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn every_reason_round_trips_through_the_tag_a_user_types() {
        for reason in Reason::ALL {
            assert_eq!(Reason::parse(reason.tag()).unwrap(), reason);
            assert_eq!(
                serde_json::to_value(reason).unwrap(),
                serde_json::Value::String(reason.tag().to_owned()),
                "the wire and `--waive` must spell {reason:?} the same way"
            );
        }
    }

    #[test]
    fn a_reason_is_spelled_with_underscores_the_way_it_is_printed() {
        assert_eq!(Reason::LockedWorktree.tag(), "locked_worktree");
        assert!(Reason::parse("locked-worktree").is_err());
    }

    #[test]
    fn a_tag_that_is_not_a_reason_is_a_usage_failure_rather_than_a_silent_pass() {
        let error = Reason::parse("everything").unwrap_err();

        assert_eq!(error.exit_code(), crate::ExitCode::Usage);
        assert!(error.to_string().contains("everything"));
    }

    #[test]
    fn a_waiver_covers_one_reason_and_says_nothing_about_the_others() {
        let waivers = Waivers::new(vec![Reason::Uncommitted]);

        assert!(waivers.covers(Reason::Uncommitted));
        for reason in Reason::ALL
            .into_iter()
            .filter(|r| *r != Reason::Uncommitted)
        {
            assert!(
                !waivers.covers(reason),
                "there is no blanket force: {reason:?}"
            );
        }
    }

    #[test]
    fn waiving_a_reason_that_was_not_raised_is_not_an_error() {
        let refusals = vec![refusal(Reason::Uncommitted)];
        let waivers = Waivers::new(vec![Reason::Uncommitted, Reason::SourceRepoMissing]);

        assert!(waivers.unwaived(&refusals).is_empty());
    }

    #[test]
    fn the_remedy_names_exactly_the_reasons_that_were_raised_in_table_order() {
        let refusals = vec![
            refusal(Reason::Unpushed),
            refusal(Reason::Uncommitted),
            refusal(Reason::Uncommitted),
        ];
        let raised = Waivers::default().unwaived(&refusals);

        assert_eq!(waive_flags(&raised), "--waive uncommitted --waive unpushed");
    }

    #[test]
    fn nothing_raised_names_nothing() {
        assert_eq!(waive_flags(&[]), "");
    }

    fn refusal(reason: Reason) -> Refusal {
        Refusal::new(
            MemberRef::Repo(PathBuf::from("/repos/api")),
            reason,
            "feat-login has something",
        )
    }
}
