//! What a member is, how it is written, and where its worktree lands.
//!
//! A member is one worktree in a plane together with what it is a worktree
//! *of*. That second half is the [`MemberRef`]: a registered project
//! (`@codestyle`) or, for an **ad-hoc member**, an absolute path to a repo that
//! is not registered (ADR-0008). The ref is what keys every plane fan-out, and
//! it is rendered identically on the command line, in `plane.toml`, on the wire
//! and on screen — one spelling, so what a refusal prints is what you type
//! back.
//!
//! The two arms cannot collide: a project name is `[a-z0-9][a-z0-9._-]*`, so it
//! contains neither `@` nor `/`, and anything that is not one is read as a path.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::EngineError;

/// The path segment at a plane directory's root that belongs to bitplane.
pub const RESERVED_SEGMENT: &str = ".bitplane";

/// The sigil that marks the project arm of a [`MemberRef`].
pub const PROJECT_SIGIL: char = '@';

/// What a member is a worktree of.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum MemberRef {
    /// A registered project, written `@codestyle`.
    Project(ProjectName),
    /// An **ad-hoc member**: an absolute path to an unregistered repo.
    Repo(PathBuf),
}

/// A project's flat, host-unique name. Never carries the sigil: the parser
/// strips it once, at the read boundary, and it is a bare name everywhere after.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct ProjectName(String);

/// Where a member's worktree lives, relative to the plane directory root.
///
/// Derived from the member's source **once, at create time**, and never
/// recomputed — so an existing plane keeps the layout it was built with when
/// the derivation rules change.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct WorktreePath(PathBuf);

impl MemberRef {
    /// Reads the string form: `@name` is a project, anything else a path.
    pub fn parse(value: &str) -> Result<MemberRef, EngineError> {
        match value.strip_prefix(PROJECT_SIGIL) {
            Some(name) => ProjectName::parse(name).map(MemberRef::Project),
            None => Ok(MemberRef::Repo(PathBuf::from(value))),
        }
    }

    /// Where this member's worktree goes, relative to the plane directory root.
    ///
    /// The derivation is the last two segments of the source, per the layout
    /// table in `CONTEXT.md`: `git@gitlab.com:acme/codestyle.git` and
    /// `~/projects/bitplane` become `acme/codestyle` and `projects/bitplane`.
    pub fn worktree_path(&self, source: &Path) -> Result<WorktreePath, EngineError> {
        let derived = WorktreePath::derived_from(source)?;

        if derived.starts_with_reserved_segment() {
            return Err(EngineError::ReservedPathSegment {
                path: derived.to_string(),
            });
        }

        Ok(derived)
    }
}

impl ProjectName {
    /// Checks the charset. Takes the bare name: the sigil is stripped by
    /// whoever read the line it came from.
    pub fn parse(name: &str) -> Result<ProjectName, EngineError> {
        if is_well_formed(name) {
            Ok(ProjectName(name.to_owned()))
        } else {
            Err(EngineError::InvalidProjectName {
                name: name.to_owned(),
            })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl WorktreePath {
    /// Reads a `plane.toml` member key, refusing everything that would let a
    /// hand-edited file aim a later `destroy` outside the plane directory
    /// (ADR-0008).
    pub fn parse(key: &str) -> Result<WorktreePath, MalformedWorktreePath> {
        if key.is_empty() {
            return Err(MalformedWorktreePath::Empty);
        }
        if key.starts_with('/') {
            return Err(MalformedWorktreePath::Absolute);
        }
        if key.starts_with("./") {
            return Err(MalformedWorktreePath::DotSlash);
        }

        let path = PathBuf::from(key);
        for component in path.components() {
            match component {
                Component::Normal(_) => {}
                Component::ParentDir => return Err(MalformedWorktreePath::ParentDir),
                Component::CurDir => return Err(MalformedWorktreePath::DotSlash),
                Component::RootDir | Component::Prefix(_) => {
                    return Err(MalformedWorktreePath::Absolute);
                }
            }
        }

        let derived = WorktreePath(path);
        if derived.starts_with_reserved_segment() {
            return Err(MalformedWorktreePath::ReservedSegment);
        }

        Ok(derived)
    }

    /// This path under `plane_directory`.
    pub fn under(&self, plane_directory: &Path) -> PathBuf {
        plane_directory.join(&self.0)
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// The last two segments of `source`, which is the whole layout rule.
    fn derived_from(source: &Path) -> Result<WorktreePath, EngineError> {
        let segments: Vec<&std::ffi::OsStr> = source
            .components()
            .filter_map(|component| match component {
                Component::Normal(segment) => Some(segment),
                _ => None,
            })
            .collect();

        let tail = segments
            .iter()
            .rev()
            .take(2)
            .rev()
            .fold(PathBuf::new(), |path, segment| path.join(segment));

        if tail.as_os_str().is_empty() {
            return Err(EngineError::InvalidRequest {
                message: format!(
                    "{} has no name to derive a worktree path from",
                    source.display()
                ),
            });
        }

        Ok(WorktreePath(tail))
    }

    fn starts_with_reserved_segment(&self) -> bool {
        self.0
            .components()
            .next()
            .is_some_and(|first| first.as_os_str() == RESERVED_SEGMENT)
    }
}

/// Why a `plane.toml` member key is not a worktree path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MalformedWorktreePath {
    Empty,
    Absolute,
    DotSlash,
    ParentDir,
    ReservedSegment,
}

impl MalformedWorktreePath {
    /// The clause that follows the key in a parse error.
    pub fn reason(self) -> &'static str {
        match self {
            MalformedWorktreePath::Empty => "is empty",
            MalformedWorktreePath::Absolute => "is absolute",
            MalformedWorktreePath::DotSlash => "starts with ./",
            MalformedWorktreePath::ParentDir => "contains ..",
            MalformedWorktreePath::ReservedSegment => "begins with .bitplane, which is reserved",
        }
    }
}

impl fmt::Display for MalformedWorktreePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason())
    }
}

impl std::error::Error for MalformedWorktreePath {}

impl From<WorktreePath> for String {
    fn from(path: WorktreePath) -> String {
        path.to_string()
    }
}

impl TryFrom<String> for WorktreePath {
    type Error = MalformedWorktreePath;

    fn try_from(key: String) -> Result<WorktreePath, MalformedWorktreePath> {
        WorktreePath::parse(&key)
    }
}

impl fmt::Display for MemberRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MemberRef::Project(name) => write!(f, "{PROJECT_SIGIL}{name}"),
            MemberRef::Repo(path) => f.write_str(&path.display().to_string()),
        }
    }
}

impl fmt::Display for ProjectName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Rendered with forward slashes, which is what `plane.toml` stores. Identical
/// to `Path`'s own rendering on every target ADR-0001 builds for.
impl fmt::Display for WorktreePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.display().to_string())
    }
}

impl From<MemberRef> for String {
    fn from(member: MemberRef) -> String {
        member.to_string()
    }
}

impl TryFrom<String> for MemberRef {
    type Error = EngineError;

    fn try_from(value: String) -> Result<MemberRef, EngineError> {
        MemberRef::parse(&value)
    }
}

impl From<ProjectName> for String {
    fn from(name: ProjectName) -> String {
        name.0
    }
}

impl TryFrom<String> for ProjectName {
    type Error = EngineError;

    fn try_from(name: String) -> Result<ProjectName, EngineError> {
        ProjectName::parse(&name)
    }
}

fn is_well_formed(name: &str) -> bool {
    let mut characters = name.chars();

    let Some(first) = characters.next() else {
        return false;
    };

    (first.is_ascii_lowercase() || first.is_ascii_digit())
        && characters.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '_' | '-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_member_ref_round_trips_through_the_string_the_file_stores() {
        let cases = [
            "@codestyle",
            "@api",
            "/Users/alfonz/projects/bitplane",
            "/tmp/repo",
        ];

        for value in cases {
            let member = MemberRef::parse(value).unwrap();
            assert_eq!(member.to_string(), value);
        }
    }

    #[test]
    fn the_sigil_is_stripped_once_and_never_carried() {
        let MemberRef::Project(name) = MemberRef::parse("@codestyle").unwrap() else {
            panic!("expected a project");
        };

        assert_eq!(name.as_str(), "codestyle");
        assert!(!name.as_str().contains(PROJECT_SIGIL));
    }

    #[test]
    fn a_sigil_on_something_that_is_not_a_project_name_is_refused() {
        for value in ["@", "@Codestyle", "@acme/codestyle", "@-api"] {
            assert!(
                matches!(
                    MemberRef::parse(value),
                    Err(EngineError::InvalidProjectName { .. })
                ),
                "{value} should not be a project"
            );
        }
    }

    #[test]
    fn the_worktree_path_is_the_last_two_segments_of_the_source() {
        let cases = [
            ("/Users/alfonz/projects/bitplane", "projects/bitplane"),
            ("/srv/acme/codestyle", "acme/codestyle"),
            ("/repo", "repo"),
        ];

        for (source, expected) in cases {
            let member = MemberRef::Repo(PathBuf::from(source));
            assert_eq!(
                member.worktree_path(Path::new(source)).unwrap().to_string(),
                expected,
                "for {source}"
            );
        }
    }

    #[test]
    fn a_source_landing_in_the_reserved_segment_is_refused() {
        let source = Path::new("/Users/alfonz/.bitplane/api");
        let member = MemberRef::Repo(source.to_path_buf());

        let error = member.worktree_path(source).unwrap_err();

        assert!(
            matches!(error, EngineError::ReservedPathSegment { .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn a_well_formed_member_key_parses() {
        for key in ["acme/codestyle", "projects/bitplane", "api", "a.b/c_d-e"] {
            assert_eq!(
                WorktreePath::parse(key).unwrap().to_string(),
                key,
                "for {key}"
            );
        }
    }

    #[test]
    fn every_dangerous_member_key_is_refused_by_name() {
        let cases = [
            ("", MalformedWorktreePath::Empty),
            ("/acme/codestyle", MalformedWorktreePath::Absolute),
            ("./acme", MalformedWorktreePath::DotSlash),
            ("../outside", MalformedWorktreePath::ParentDir),
            ("acme/../../outside", MalformedWorktreePath::ParentDir),
            (".bitplane/api", MalformedWorktreePath::ReservedSegment),
            (".bitplane", MalformedWorktreePath::ReservedSegment),
        ];

        for (key, expected) in cases {
            assert_eq!(WorktreePath::parse(key), Err(expected), "for {key:?}");
        }
    }

    #[test]
    fn a_member_key_resolves_under_the_plane_directory_it_belongs_to() {
        let key = WorktreePath::parse("acme/codestyle").unwrap();

        assert_eq!(
            key.under(Path::new("/planes/bp-a3f9c2e1")),
            PathBuf::from("/planes/bp-a3f9c2e1/acme/codestyle")
        );
    }
}
