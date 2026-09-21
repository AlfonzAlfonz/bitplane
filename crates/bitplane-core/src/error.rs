//! The error envelope, and the typed failures that produce it.
//!
//! The envelope is constructed here, in the core crate, so the JSON written to
//! stderr is byte-identical across the CLI, a future MCP surface and the SSH
//! servant (ADR-0003). stdout carries results only.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::exit::ExitCode;
use crate::git::{GitVersion, MINIMUM_GIT_VERSION};

/// What every bitplane failure looks like on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    /// A stable machine tag — matched on by scripts, never rendered to a user.
    pub error: String,
    /// The code the process exits with.
    pub code: ExitCode,
    /// One human sentence saying what went wrong.
    pub message: String,
    /// The individual things that were wrong, where the failure has more than
    /// one. A refusal across four members is four problems.
    pub problems: Vec<Problem>,
    /// What to do about it, where bitplane knows.
    pub remedy: Option<String>,
}

/// One of the things that was wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    /// What the problem is about — a member, a plane id, a path — rendered for
    /// display. `None` where the problem is about the operation as a whole.
    pub subject: Option<String>,
    /// One human sentence about this subject.
    pub message: String,
}

/// Everything that can go wrong inside the engine.
///
/// Serialisable in both directions, because a fan-out row carries one of these
/// as data rather than as an envelope (ADR-0003).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
#[non_exhaustive]
pub enum EngineError {
    /// No `git` on `PATH`.
    GitMissing,
    /// `git` is on `PATH` but would not run.
    GitUnusable { message: String },
    /// `git --version` printed something that is not a version.
    GitVersionUnreadable { reported: String },
    /// `git` is older than bitplane's floor (ADR-0001).
    GitTooOld {
        found: GitVersion,
        required: GitVersion,
    },
    /// The request could not be acted on as given.
    InvalidRequest { message: String },
    /// A lock could not be taken in time. The one failure that means "try
    /// again" rather than "this did not work".
    LockTimeout { object: PathBuf },
    /// The filesystem refused.
    Io { path: PathBuf, message: String },
}

impl EngineError {
    /// The stable machine tag for this failure.
    pub fn tag(&self) -> &'static str {
        match self {
            EngineError::GitMissing => "git_missing",
            EngineError::GitUnusable { .. } => "git_unusable",
            EngineError::GitVersionUnreadable { .. } => "git_version_unreadable",
            EngineError::GitTooOld { .. } => "git_too_old",
            EngineError::InvalidRequest { .. } => "invalid_request",
            EngineError::LockTimeout { .. } => "lock_timeout",
            EngineError::Io { .. } => "io",
        }
    }

    /// The code the process exits with when this failure is what stopped it.
    pub fn exit_code(&self) -> ExitCode {
        match self {
            EngineError::GitMissing
            | EngineError::GitUnusable { .. }
            | EngineError::GitVersionUnreadable { .. }
            | EngineError::GitTooOld { .. } => ExitCode::PrerequisiteMissing,
            EngineError::InvalidRequest { .. } => ExitCode::Usage,
            EngineError::LockTimeout { .. } => ExitCode::Busy,
            EngineError::Io { .. } => ExitCode::Failure,
        }
    }

    /// Renders this failure as the envelope written to stderr.
    pub fn envelope(&self) -> ErrorEnvelope {
        ErrorEnvelope {
            error: self.tag().to_owned(),
            code: self.exit_code(),
            message: self.to_string(),
            problems: self.problems(),
            remedy: self.remedy(),
        }
    }

    fn problems(&self) -> Vec<Problem> {
        match self {
            EngineError::LockTimeout { object } => {
                vec![Problem::about_path(object, "is locked by another process")]
            }
            EngineError::Io { path, message } => vec![Problem::about_path(path, message)],
            _ => Vec::new(),
        }
    }

    fn remedy(&self) -> Option<String> {
        match self {
            EngineError::GitMissing => Some(format!(
                "Install git {MINIMUM_GIT_VERSION} or newer and put it on PATH."
            )),
            EngineError::GitUnusable { .. } | EngineError::GitVersionUnreadable { .. } => Some(
                format!("Check that `git --version` reports git {MINIMUM_GIT_VERSION} or newer."),
            ),
            EngineError::GitTooOld { required, .. } => {
                Some(format!("Upgrade git to {required} or newer."))
            }
            EngineError::LockTimeout { .. } => {
                Some("Another bitplane process holds it; retry once that one finishes.".to_owned())
            }
            EngineError::InvalidRequest { .. } | EngineError::Io { .. } => None,
        }
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::GitMissing => write!(
                f,
                "git was not found on PATH; bitplane requires git {MINIMUM_GIT_VERSION} or newer"
            ),
            EngineError::GitUnusable { message } => write!(
                f,
                "git could not be run ({message}); bitplane requires git {MINIMUM_GIT_VERSION} or newer"
            ),
            EngineError::GitVersionUnreadable { reported } => write!(
                f,
                "git's version could not be read from {reported:?}; bitplane requires git {MINIMUM_GIT_VERSION} or newer"
            ),
            EngineError::GitTooOld { found, required } => {
                write!(
                    f,
                    "git {found} is too old; bitplane requires git {required} or newer"
                )
            }
            EngineError::InvalidRequest { message } => write!(f, "{message}"),
            EngineError::LockTimeout { object } => {
                write!(f, "timed out waiting for the lock on {}", object.display())
            }
            EngineError::Io { path, message } => {
                write!(f, "{}: {message}", path.display())
            }
        }
    }
}

impl Error for EngineError {}

impl ErrorEnvelope {
    /// A usage failure — a malformed command line, or a name already taken.
    pub fn usage(error: impl Into<String>, message: impl Into<String>) -> ErrorEnvelope {
        ErrorEnvelope {
            error: error.into(),
            code: ExitCode::Usage,
            message: message.into(),
            problems: Vec::new(),
            remedy: None,
        }
    }

    /// Attaches the individual things that were wrong.
    pub fn with_problems(mut self, problems: Vec<Problem>) -> ErrorEnvelope {
        self.problems = problems;
        self
    }

    /// Attaches what to do about it.
    pub fn with_remedy(mut self, remedy: impl Into<String>) -> ErrorEnvelope {
        self.remedy = Some(remedy.into());
        self
    }

    /// The single JSON line written to stderr.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("an error envelope is always serialisable")
    }
}

impl Problem {
    /// A problem with the operation as a whole.
    pub fn new(message: impl Into<String>) -> Problem {
        Problem {
            subject: None,
            message: message.into(),
        }
    }

    /// A problem with one named thing.
    pub fn about(subject: impl Into<String>, message: impl Into<String>) -> Problem {
        Problem {
            subject: Some(subject.into()),
            message: message.into(),
        }
    }

    /// A problem with one path. The path is rendered only here, at the wire
    /// boundary; it is held as a path type everywhere else (ADR-0001).
    pub fn about_path(path: &Path, message: impl Into<String>) -> Problem {
        Problem::about(path.display().to_string(), message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_exit_code_but_ok_and_drift_is_reachable_from_a_typed_failure() {
        let cases = [
            (EngineError::GitMissing, ExitCode::PrerequisiteMissing),
            (
                EngineError::InvalidRequest {
                    message: "nope".to_owned(),
                },
                ExitCode::Usage,
            ),
            (
                EngineError::LockTimeout {
                    object: PathBuf::from("/planes/bp-a3f9c2e1/.bitplane/lock"),
                },
                ExitCode::Busy,
            ),
            (
                EngineError::Io {
                    path: PathBuf::from("/planes/bp-a3f9c2e1/plane.toml"),
                    message: "permission denied".to_owned(),
                },
                ExitCode::Failure,
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.exit_code(), expected, "for {error:?}");
            assert_eq!(error.envelope().code, expected, "for {error:?}");
        }
    }

    #[test]
    fn the_envelope_is_the_five_contract_fields_in_order() {
        // Asserted on the exact bytes: the envelope has to serialise identically
        // across the CLI, an MCP surface and the SSH servant (ADR-0003).
        let envelope = ErrorEnvelope::usage("no_command", "no command given")
            .with_remedy("Run `bp --help` to see what bp can do.");

        assert_eq!(
            envelope.to_json(),
            concat!(
                r#"{"error":"no_command","code":2,"message":"no command given","#,
                r#""problems":[],"remedy":"Run `bp --help` to see what bp can do.""#,
                "}"
            )
        );
    }

    #[test]
    fn a_typed_failure_renders_its_subject_as_a_problem() {
        let envelope = EngineError::LockTimeout {
            object: PathBuf::from("/planes/bp-a3f9c2e1/.bitplane/lock"),
        }
        .envelope();

        let json: serde_json::Value = serde_json::from_str(&envelope.to_json()).unwrap();
        assert_eq!(json["error"], "lock_timeout");
        assert_eq!(json["code"], 5);
        assert_eq!(
            json["problems"][0]["subject"],
            "/planes/bp-a3f9c2e1/.bitplane/lock"
        );
        assert!(json["remedy"].is_string());
    }

    /// Every variant, so the two tests below cover the whole enum rather than
    /// whichever variants were current when they were written.
    fn every_failure() -> Vec<EngineError> {
        vec![
            EngineError::GitMissing,
            EngineError::GitUnusable {
                message: "exited 1".to_owned(),
            },
            EngineError::GitVersionUnreadable {
                reported: "not a version at all".to_owned(),
            },
            EngineError::GitTooOld {
                found: GitVersion::new(2, 35, 9),
                required: MINIMUM_GIT_VERSION,
            },
            EngineError::InvalidRequest {
                message: "nope".to_owned(),
            },
            EngineError::LockTimeout {
                object: PathBuf::from("/planes/bp-a3f9c2e1/.bitplane/lock"),
            },
            EngineError::Io {
                path: PathBuf::from("/planes/bp-a3f9c2e1/plane.toml"),
                message: "permission denied".to_owned(),
            },
        ]
    }

    #[test]
    fn the_envelope_tag_is_the_tag_the_wire_uses() {
        // The tag is spelled twice — in `tag()` and in serde's `tag = "error"`
        // — so a client matching on a fan-out row and one matching on stderr
        // must be matching on the same string.
        for error in every_failure() {
            let on_the_wire = serde_json::to_value(&error).unwrap();

            assert_eq!(
                on_the_wire["error"],
                error.tag(),
                "`tag()` and the serde tag disagree for {error:?}"
            );
            assert_eq!(error.envelope().error, error.tag(), "for {error:?}");
        }
    }

    #[test]
    fn every_failure_renders_a_message_and_a_code() {
        for error in every_failure() {
            let envelope = error.envelope();

            assert!(!envelope.message.is_empty(), "for {error:?}");
            assert_ne!(envelope.code, ExitCode::Ok, "for {error:?}");
            assert_ne!(envelope.code, ExitCode::Drift, "for {error:?}");
        }
    }

    #[test]
    fn a_typed_failure_round_trips_over_the_wire() {
        let error = EngineError::GitTooOld {
            found: GitVersion::new(2, 35, 9),
            required: MINIMUM_GIT_VERSION,
        };

        let json = serde_json::to_string(&error).unwrap();
        assert_eq!(
            serde_json::from_str::<EngineError>(&json).unwrap(),
            error,
            "round-tripped from {json}"
        );
    }

    #[test]
    fn a_usage_envelope_carries_the_usage_code() {
        let envelope = ErrorEnvelope::usage("no_command", "no command given")
            .with_remedy("Run `bp --help` to see what bp can do.");

        assert_eq!(envelope.code, ExitCode::Usage);
        assert!(envelope.problems.is_empty());
        assert_eq!(
            envelope.remedy.as_deref(),
            Some("Run `bp --help` to see what bp can do.")
        );
    }
}
