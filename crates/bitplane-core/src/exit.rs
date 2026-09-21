//! The six exit codes.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Every code `bp` can exit with — wkt's set, plus `Busy` (ADR-0003).
///
/// `Busy` is bitplane's addition, for a lock timeout: it is neither a usage
/// error nor a failure of the operation but "try again", and a script needs to
/// tell that apart to know whether retrying is sane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ExitCode {
    /// The command did what was asked.
    Ok = 0,
    /// A typed failure: the operation ran and did not succeed.
    Failure = 1,
    /// The request was malformed, or what it asked to create already exists.
    Usage = 2,
    /// The command succeeded and reported drift. Drift is a state bitplane
    /// reports, not an error.
    Drift = 3,
    /// A prerequisite is missing — today, a git that is absent or older than
    /// [`crate::MINIMUM_GIT_VERSION`].
    PrerequisiteMissing = 4,
    /// A lock on a plane or a source repo could not be taken in time.
    Busy = 5,
}

impl ExitCode {
    /// The number the process exits with.
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

impl From<ExitCode> for u8 {
    fn from(code: ExitCode) -> u8 {
        code.as_u8()
    }
}

impl From<ExitCode> for std::process::ExitCode {
    fn from(code: ExitCode) -> std::process::ExitCode {
        std::process::ExitCode::from(code.as_u8())
    }
}

/// Serialised as the bare number, so a client reading the envelope sees the
/// same value the process exited with.
impl Serialize for ExitCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(self.as_u8())
    }
}

impl<'de> Deserialize<'de> for ExitCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<ExitCode, D::Error> {
        let code = u8::deserialize(deserializer)?;
        ExitCode::from_u8(code)
            .ok_or_else(|| serde::de::Error::custom(format!("{code} is not a bitplane exit code")))
    }
}

impl ExitCode {
    fn from_u8(code: u8) -> Option<ExitCode> {
        match code {
            0 => Some(ExitCode::Ok),
            1 => Some(ExitCode::Failure),
            2 => Some(ExitCode::Usage),
            3 => Some(ExitCode::Drift),
            4 => Some(ExitCode::PrerequisiteMissing),
            5 => Some(ExitCode::Busy),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_code_is_defined_and_round_trips() {
        let all = [
            (ExitCode::Ok, 0),
            (ExitCode::Failure, 1),
            (ExitCode::Usage, 2),
            (ExitCode::Drift, 3),
            (ExitCode::PrerequisiteMissing, 4),
            (ExitCode::Busy, 5),
        ];

        for (code, number) in all {
            assert_eq!(code.as_u8(), number);
            assert_eq!(serde_json::to_string(&code).unwrap(), number.to_string());
            assert_eq!(
                serde_json::from_str::<ExitCode>(&number.to_string()).unwrap(),
                code
            );
        }
    }

    #[test]
    fn a_number_outside_the_set_is_not_an_exit_code() {
        assert!(serde_json::from_str::<ExitCode>("6").is_err());
    }
}
