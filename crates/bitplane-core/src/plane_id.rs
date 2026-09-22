//! The name of a plane, which is also the name of its directory.
//!
//! A plane id is **mutable by design** — a wall of `bp-a3f9c2e1` directories is
//! impossible to navigate, and renaming is the escape hatch — so nothing
//! outside bitplane may cache one and expect it to resolve later.
//!
//! The `bp-` prefix is reserved for generated ids and rejected as a user-chosen
//! one, so *"starts with `bp-`"* stays a reliable signal that nobody chose this
//! name. User-chosen ids are lowercase only, so two can never collide on a
//! case-insensitive filesystem.

use std::fmt;
use std::fs::File;
use std::io::Read;

use serde::{Deserialize, Serialize};

use crate::error::EngineError;

/// The prefix bitplane puts on the ids it generates, and refuses on the ids it
/// is given.
pub const GENERATED_ID_PREFIX: &str = "bp-";

/// The longest a plane id may be.
pub const MAX_PLANE_ID_LENGTH: usize = 64;

/// How many times a generated id may collide before bitplane concludes that
/// something other than collision is wrong (ADR-0002).
pub const GENERATED_ID_ATTEMPTS: usize = 5;

/// A plane id: lowercase `[a-z0-9][a-z0-9._-]*`, 64 characters at most.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct PlaneId(String);

impl PlaneId {
    /// A user-chosen id, checked. Rejects the reserved generated-id prefix
    /// before the charset, so `bp-Auth` is reported as reserved rather than as
    /// badly spelled — the prefix is the rule the user has to learn.
    pub fn chosen(id: &str) -> Result<PlaneId, EngineError> {
        if id.starts_with(GENERATED_ID_PREFIX) {
            return Err(EngineError::ReservedPlaneId);
        }
        PlaneId::parse(id)
    }

    /// An id read from a file or generated. Checks the charset and nothing
    /// else: `plane.toml` legitimately holds generated ids.
    pub fn parse(id: &str) -> Result<PlaneId, EngineError> {
        if is_well_formed(id) {
            Ok(PlaneId(id.to_owned()))
        } else {
            Err(EngineError::InvalidPlaneId { id: id.to_owned() })
        }
    }

    /// A fresh `bp-<hex8>`.
    ///
    /// There is no allocator and no coordination: collisions are detected by
    /// attempting the directory create and retrying (ADR-0002), so this only
    /// has to be unlikely to repeat, not guaranteed unique.
    pub fn generate() -> Result<PlaneId, EngineError> {
        let mut bytes = [0u8; 4];
        random_bytes(&mut bytes)?;

        let mut id = String::from(GENERATED_ID_PREFIX);
        for byte in bytes {
            id.push_str(&format!("{byte:02x}"));
        }

        Ok(PlaneId(id))
    }

    /// Whether bitplane generated this id rather than being given it.
    pub fn is_generated(&self) -> bool {
        self.0.starts_with(GENERATED_ID_PREFIX)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PlaneId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<PlaneId> for String {
    fn from(id: PlaneId) -> String {
        id.0
    }
}

impl TryFrom<String> for PlaneId {
    type Error = EngineError;

    fn try_from(id: String) -> Result<PlaneId, EngineError> {
        PlaneId::parse(&id)
    }
}

/// `[a-z0-9][a-z0-9._-]*`, 64 characters at most.
fn is_well_formed(id: &str) -> bool {
    let mut characters = id.chars();

    let Some(first) = characters.next() else {
        return false;
    };

    id.len() <= MAX_PLANE_ID_LENGTH
        && (first.is_ascii_lowercase() || first.is_ascii_digit())
        && characters.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '_' | '-')
        })
}

/// The system's entropy, read directly.
///
/// `/dev/urandom` rather than a crate: ADR-0001 is POSIX-only, and the only
/// consumer is an id whose uniqueness is enforced by `mkdir` regardless.
fn random_bytes(into: &mut [u8]) -> Result<(), EngineError> {
    let source = std::path::Path::new("/dev/urandom");

    File::open(source)
        .and_then(|mut handle| handle.read_exact(into))
        .map_err(|err| EngineError::io(source, err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_well_formed_id_is_accepted() {
        for id in [
            "auth-work",
            "a",
            "0",
            "work.2",
            "a_b",
            "x".repeat(64).as_str(),
        ] {
            assert!(PlaneId::chosen(id).is_ok(), "{id} should be a valid id");
        }
    }

    #[test]
    fn a_malformed_id_is_rejected_by_name() {
        for id in [
            "",
            "Auth_Work",
            "-leading-dash",
            ".leading-dot",
            "_leading-underscore",
            "has space",
            "has/slash",
            "has@sigil",
            "x".repeat(65).as_str(),
        ] {
            let error = PlaneId::chosen(id).unwrap_err();
            assert!(
                matches!(error, EngineError::InvalidPlaneId { .. }),
                "{id} should be invalid, got {error:?}"
            );
        }
    }

    #[test]
    fn the_generated_prefix_is_reserved_from_a_user() {
        for id in ["bp-a3f9c2e1", "bp-", "bp-mine"] {
            assert_eq!(
                PlaneId::chosen(id).unwrap_err(),
                EngineError::ReservedPlaneId,
                "for {id}"
            );
        }
    }

    #[test]
    fn the_reserved_prefix_is_still_readable_from_a_file() {
        let id = PlaneId::parse("bp-a3f9c2e1").unwrap();

        assert!(id.is_generated());
    }

    #[test]
    fn a_generated_id_is_the_prefix_and_eight_hex_digits() {
        let id = PlaneId::generate().unwrap();

        assert!(id.is_generated(), "got {id}");
        assert_eq!(id.as_str().len(), GENERATED_ID_PREFIX.len() + 8, "got {id}");
        assert!(
            id.as_str()[GENERATED_ID_PREFIX.len()..]
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "got {id}"
        );
        assert!(PlaneId::parse(id.as_str()).is_ok(), "got {id}");
    }

    #[test]
    fn two_generated_ids_differ() {
        let ids: Vec<String> = (0..16)
            .map(|_| PlaneId::generate().unwrap().to_string())
            .collect();

        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();

        assert_eq!(unique.len(), ids.len(), "generated ids repeated: {ids:?}");
    }
}
