//! What a read found wrong, and how hard it looked.
//!
//! There is no reconcile action and nothing stores staleness: ADR-0002 forbids
//! a read writing, so health is **recomputed on every read** and rides on
//! `plane_list` and `plane_show` as a field (ADR-0003).
//!
//! [`HealthCheck`] is what keeps that affordable. `bp list` runs constantly and
//! must not spawn a git per member, so the default tier is filesystem-only and
//! the one finding that needs a git process — git's own `prunable` — is opted
//! into.
//!
//! A [`Finding`] is a typed value with a tag, a sentence and a remedy, exactly
//! like [`crate::EngineError`], and for the same reason: the words are built
//! here so every surface reports the same finding in the same sentence.
//!
//! **Drift is never the user's work.** Uncommitted changes, untracked files, a
//! branch switched, a rebase in progress — none of that is here, because
//! bitplane owns a worktree's existence and location and nothing else
//! (ADR-0006). Every finding below is about something bitplane owns, or about a
//! file bitplane needs and could not read.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::EngineError;
use crate::member::{MemberRef, WorktreePath};
use crate::plane_id::PlaneId;

/// How hard a read looks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum HealthCheck {
    /// Membership and nothing else.
    None,
    /// Filesystem only: no git process is started.
    #[default]
    Cheap,
    /// The cheap findings, plus git's own `prunable`. One git invocation per
    /// source repo.
    Full,
}

/// What a read found about one plane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneHealth {
    /// The tier this was computed at, so a caller can tell "nothing found" from
    /// "nothing looked for".
    pub checked: HealthCheck,
    /// In the order a row prints them: the plane's own findings first, then one
    /// per member in plane file order.
    pub findings: Vec<Finding>,
}

/// One thing a read found wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "finding", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Finding {
    /// The plane file names a member whose worktree directory is not there.
    MemberWorktreeMissing {
        member: MemberRef,
        path: WorktreePath,
    },
    /// The **incomplete latch** is still set: this plane was claimed and never
    /// finished being built. It says something no other signal can — that the
    /// plane *never worked*, as opposed to having worked and since lost a
    /// worktree — which is what makes it safe to discard (ADR-0004).
    CreateNeverCompleted {
        /// The plane, because the remedy names it.
        plane: String,
        /// When the claim was made, where the filesystem can say.
        started: Option<String>,
        /// How long ago that was, worded at the moment of the scan: "3 days" is
        /// the part a person acts on, and a timestamp alone is not.
        ago: Option<String>,
    },
    /// The repository a member is a worktree of is gone.
    SourceRepoMissing { member: MemberRef, source: PathBuf },
    /// `plane.toml`'s `id` disagrees with the directory name.
    ///
    /// The directory name **is** the plane's identity, so the stored id is a
    /// self-check and a disagreement is a positive signal — the plane was moved
    /// outside bitplane — rather than a problem to paper over (ADR-0008).
    IdMismatch {
        recorded: PlaneId,
        directory: String,
    },
    /// git's own word: it records the worktree at a path that is no longer
    /// there. A plain `mv` within the plane produces exactly this.
    Prunable {
        member: MemberRef,
        path: WorktreePath,
    },
    /// A file bitplane needs did not parse. The listing printed a row and
    /// carried on; nothing was repaired, because a file bitplane cannot read is
    /// one it has no business rewriting.
    Unreadable { error: EngineError },
}

impl PlaneHealth {
    /// A plane nothing was found wrong with.
    pub fn sound(checked: HealthCheck) -> PlaneHealth {
        PlaneHealth {
            checked,
            findings: Vec::new(),
        }
    }

    /// Whether this plane reported anything. A `true` here is what makes the
    /// command exit `3`.
    pub fn has_findings(&self) -> bool {
        !self.findings.is_empty()
    }

    /// The findings about one member, in order.
    pub fn about(&self, member: &MemberRef) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(move |finding| finding.member() == Some(member))
    }

    /// The findings about the plane as a whole rather than about one member.
    pub fn about_the_plane(&self) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(|finding| finding.member().is_none())
    }
}

impl Finding {
    /// The stable machine tag, matching the serialised `finding` field.
    pub fn tag(&self) -> &'static str {
        match self {
            Finding::MemberWorktreeMissing { .. } => "member_worktree_missing",
            Finding::CreateNeverCompleted { .. } => "create_never_completed",
            Finding::SourceRepoMissing { .. } => "source_repo_missing",
            Finding::IdMismatch { .. } => "id_mismatch",
            Finding::Prunable { .. } => "prunable",
            Finding::Unreadable { .. } => "unreadable",
        }
    }

    /// The member this is about, where it is about one. `None` means the plane
    /// itself, which is what decides whether a row or a line prints it.
    pub fn member(&self) -> Option<&MemberRef> {
        match self {
            Finding::MemberWorktreeMissing { member, .. }
            | Finding::SourceRepoMissing { member, .. }
            | Finding::Prunable { member, .. } => Some(member),
            Finding::CreateNeverCompleted { .. }
            | Finding::IdMismatch { .. }
            | Finding::Unreadable { .. } => None,
        }
    }

    /// The few words a fan-out row prints in its note column, where the whole
    /// sentence would not fit beside a branch.
    pub fn label(&self) -> &'static str {
        match self {
            Finding::MemberWorktreeMissing { .. } => "worktree missing",
            Finding::CreateNeverCompleted { .. } => "create never completed",
            Finding::SourceRepoMissing { .. } => "source repo missing",
            Finding::IdMismatch { .. } => "id mismatch",
            Finding::Prunable { .. } => "prunable",
            Finding::Unreadable { .. } => "unreadable",
        }
    }

    /// What to do about it. Every finding names the command that fixes it;
    /// a read reports and never repairs.
    pub fn remedy(&self) -> Option<String> {
        match self {
            Finding::MemberWorktreeMissing { member, .. } => Some(format!(
                "bp rm {member} drops it; bp add {member}:<branch> puts it back"
            )),
            Finding::CreateNeverCompleted { plane, .. } => Some(format!(
                "nothing in it is yours; bp destroy -p {plane} clears it"
            )),
            Finding::SourceRepoMissing { member, .. } => Some(format!(
                "bp rm {member} drops it; restoring the repository also fixes it"
            )),
            Finding::IdMismatch { .. } => Some(
                "the plane was moved outside bp, or a rename was interrupted; \
                 bp repair reconnects the worktrees and settles the id"
                    .to_owned(),
            ),
            Finding::Prunable { .. } => {
                Some("bp repair rewrites git's record of where it is".to_owned())
            }
            // Whatever the parse failure itself says — `Legal keys are …` for a
            // bad key, and nothing where bitplane has no advice to give.
            Finding::Unreadable { error } => error.envelope().remedy,
        }
    }
}

/// One sentence saying what was found. The subject is rendered beside it rather
/// than inside it, so a row and a JSON field carry the same words.
impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Finding::MemberWorktreeMissing { path, .. } => {
                write!(f, "the worktree at {path} is not there")
            }
            Finding::CreateNeverCompleted { ago, .. } => match ago {
                Some(ago) => write!(f, "create never completed, started {ago} ago"),
                None => f.write_str("create never completed"),
            },
            Finding::SourceRepoMissing { source, .. } => {
                write!(f, "the repository at {} is not there", source.display())
            }
            Finding::IdMismatch {
                recorded,
                directory,
            } => write!(
                f,
                "plane.toml says {recorded}; this directory is {directory}"
            ),
            Finding::Prunable { path, .. } => write!(
                f,
                "git records the worktree at {path} at a path that is no longer there"
            ),
            Finding::Unreadable { error } => error.fmt(f),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_cheap_tier_is_the_default_because_list_runs_constantly() {
        assert_eq!(HealthCheck::default(), HealthCheck::Cheap);
    }

    #[test]
    fn every_finding_says_what_is_wrong_and_what_fixes_it() {
        for finding in every_finding() {
            assert!(
                !finding.to_string().is_empty(),
                "{} says nothing",
                finding.tag()
            );
            assert!(
                finding.remedy().is_some(),
                "{} names no way out",
                finding.tag()
            );
            assert!(!finding.label().is_empty());
        }
    }

    #[test]
    fn a_finding_serialises_under_the_tag_it_reports() {
        for finding in every_finding() {
            let json: serde_json::Value = serde_json::to_value(&finding).unwrap();

            assert_eq!(json["finding"], finding.tag());
            assert_eq!(
                serde_json::from_value::<Finding>(json).unwrap(),
                finding,
                "{} does not round trip",
                finding.tag()
            );
        }
    }

    #[test]
    fn a_finding_about_a_member_is_told_from_one_about_the_plane() {
        let member = MemberRef::parse("@api").unwrap();
        let health = PlaneHealth {
            checked: HealthCheck::Cheap,
            findings: vec![
                Finding::IdMismatch {
                    recorded: PlaneId::parse("bp-a3f9c2e1").unwrap(),
                    directory: "auth-work".to_owned(),
                },
                Finding::MemberWorktreeMissing {
                    member: member.clone(),
                    path: WorktreePath::parse("acme/api").unwrap(),
                },
            ],
        };

        assert!(health.has_findings());
        assert_eq!(health.about(&member).count(), 1);
        assert_eq!(health.about_the_plane().count(), 1);
    }

    #[test]
    fn an_unreadable_plane_file_carries_the_parse_failures_own_words() {
        let error = EngineError::ParseError {
            path: PathBuf::from("/planes/broken/plane.toml"),
            message: "unknown key \"status\"".to_owned(),
            legal_keys: ["version", "id", "members"].map(str::to_owned).to_vec(),
        };

        let finding = Finding::Unreadable {
            error: error.clone(),
        };

        assert_eq!(finding.to_string(), error.to_string());
        assert_eq!(finding.remedy(), error.envelope().remedy);
    }

    /// Every variant, so the tests above cover the whole enum rather than
    /// whichever variants were current when they were written.
    fn every_finding() -> Vec<Finding> {
        let member = MemberRef::parse("@api").unwrap();
        let path = WorktreePath::parse("acme/api").unwrap();

        vec![
            Finding::MemberWorktreeMissing {
                member: member.clone(),
                path: path.clone(),
            },
            Finding::CreateNeverCompleted {
                plane: "auth-work".to_owned(),
                started: Some("2026-09-18T09:41:00Z".to_owned()),
                ago: Some("3 days".to_owned()),
            },
            Finding::SourceRepoMissing {
                member: member.clone(),
                source: Path::new("/Users/alfonz/projects/api").to_path_buf(),
            },
            Finding::IdMismatch {
                recorded: PlaneId::parse("bp-a3f9c2e1").unwrap(),
                directory: "auth-work".to_owned(),
            },
            Finding::Prunable { member, path },
            Finding::Unreadable {
                error: EngineError::ParseError {
                    path: PathBuf::from("/planes/broken/plane.toml"),
                    message: "unknown key \"status\"".to_owned(),
                    legal_keys: ["version", "id", "members"].map(str::to_owned).to_vec(),
                },
            },
        ]
    }
}
