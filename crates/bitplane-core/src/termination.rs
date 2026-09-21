//! How an invocation ends.

use crate::error::ErrorEnvelope;
use crate::exit::ExitCode;

/// What one invocation amounts to, and therefore what the process exits with.
///
/// Drift is a state bitplane reports rather than an error (ADR-0003), so it is a
/// third arm here and not an [`ErrorEnvelope`]: the results still go to stdout,
/// and only the exit code says the plane has drifted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Termination {
    /// The command did what was asked.
    Ok,
    /// The command did what was asked and found drift.
    Drift,
    /// The command failed. The envelope is written to stderr as JSON.
    Failed(ErrorEnvelope),
}

impl Termination {
    /// The code the process exits with.
    pub fn exit_code(&self) -> ExitCode {
        match self {
            Termination::Ok => ExitCode::Ok,
            Termination::Drift => ExitCode::Drift,
            Termination::Failed(envelope) => envelope.code,
        }
    }

    /// The envelope to write to stderr, where there is one.
    pub fn envelope(&self) -> Option<&ErrorEnvelope> {
        match self {
            Termination::Ok | Termination::Drift => None,
            Termination::Failed(envelope) => Some(envelope),
        }
    }
}

impl From<ErrorEnvelope> for Termination {
    fn from(envelope: ErrorEnvelope) -> Termination {
        Termination::Failed(envelope)
    }
}

impl From<crate::error::EngineError> for Termination {
    fn from(error: crate::error::EngineError) -> Termination {
        Termination::Failed(error.envelope())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::EngineError;

    #[test]
    fn all_six_exit_codes_are_reachable_through_a_termination() {
        assert_eq!(Termination::Ok.exit_code(), ExitCode::Ok);
        assert_eq!(Termination::Drift.exit_code(), ExitCode::Drift);

        let failures = [
            (
                EngineError::Io {
                    path: "/planes/bp-a3f9c2e1/plane.toml".into(),
                    message: "permission denied".to_owned(),
                },
                ExitCode::Failure,
            ),
            (
                EngineError::InvalidRequest {
                    message: "no command given".to_owned(),
                },
                ExitCode::Usage,
            ),
            (EngineError::GitMissing, ExitCode::PrerequisiteMissing),
            (
                EngineError::LockTimeout {
                    object: "/planes/bp-a3f9c2e1/.bitplane/lock".into(),
                },
                ExitCode::Busy,
            ),
        ];

        for (error, expected) in failures {
            let termination = Termination::from(error);
            assert_eq!(termination.exit_code(), expected);
            assert!(termination.envelope().is_some());
        }
    }

    #[test]
    fn a_successful_termination_has_no_envelope() {
        assert!(Termination::Ok.envelope().is_none());
        assert!(Termination::Drift.envelope().is_none());
    }
}
