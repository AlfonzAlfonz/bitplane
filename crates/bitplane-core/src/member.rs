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
//! **The sigil, not the charset, is what keeps the two arms apart.** `@` is
//! required wherever a path is also accepted, which is the only position where
//! the ambiguity exists — so `@acme/codestyle` is a project and bare
//! `acme/codestyle` is a path, even though a project name may now contain a
//! `/` (ADR-0009). The charset argument that used to be given here — *a name
//! contains no `/`, so the arms cannot collide* — was a second, accidental
//! guarantee, and it is gone.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::EngineError;
use crate::project_dir::{BIN_DIR_NAME, REPO_DIR_NAME};

/// The path segment at a plane directory's root that belongs to bitplane.
pub const RESERVED_SEGMENT: &str = ".bitplane";

/// The sigil that marks the project arm of a [`MemberRef`].
pub const PROJECT_SIGIL: char = '@';

/// The longest a project name may be. A filesystem bound rather than a
/// modelling opinion: **depth itself is uncapped**, because a forge nests as
/// deeply as its groups do (ADR-0009).
pub const MAX_PROJECT_NAME_LENGTH: usize = 128;

/// Segments a project name may not use.
///
/// Both are legal under the segment charset and both name a directory bitplane
/// puts *inside* a project directory, which the walk that finds projects skips
/// **by name** — it must, because an interrupted `project add` leaves a
/// `repo.git` with no `project.toml` behind it. So a project called `@acme/bin`
/// would sit at a path nothing ever enters: registrable once and invisible
/// thereafter. The reservation is what makes the skip list safe; without it the
/// skip list is the thing that loses projects (ADR-0009).
///
/// `.bitplane` needs no entry here — a segment cannot start with `.`.
pub const RESERVED_NAME_SEGMENTS: [&str; 2] = [REPO_DIR_NAME, BIN_DIR_NAME];

/// What a member is a worktree of.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum MemberRef {
    /// A registered project, written `@codestyle`.
    Project(ProjectName),
    /// An **ad-hoc member**: an absolute path to an unregistered repo.
    Repo(PathBuf),
}

/// A project's host-unique name: a `/`-separated **path of segments**, each on
/// the charset ADR-0007 settled. Never carries the sigil — the parser strips it
/// once, at the read boundary, and it is a bare name everywhere after.
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
    /// Checks the charset, segment by segment. Takes the bare name: the sigil
    /// is stripped by whoever read the line it came from.
    ///
    /// A leading or trailing `/`, an empty segment and a `.` or `..` segment
    /// are all refused. They fall out of the charset rather than needing rules
    /// of their own — a segment cannot start with `.` — but a name is a path on
    /// disk now, and a path that can say `..` is a path that can escape.
    ///
    /// **Strict everywhere, always.** The silent lowercasing ADR-0009 allows
    /// belongs to name *derivation* and happens before this is called, so a
    /// hand-edited `project.toml` carrying a capital is still a parse error and
    /// never a file bitplane quietly rewrites.
    pub fn parse(name: &str) -> Result<ProjectName, EngineError> {
        if name.is_empty() || name.chars().count() > MAX_PROJECT_NAME_LENGTH {
            return Err(EngineError::InvalidProjectName {
                name: name.to_owned(),
            });
        }

        for segment in name.split('/') {
            if !ProjectName::is_segment(segment) {
                return Err(EngineError::InvalidProjectName {
                    name: name.to_owned(),
                });
            }

            if RESERVED_NAME_SEGMENTS.contains(&segment) {
                return Err(EngineError::ReservedNameSegment {
                    name: name.to_owned(),
                    segment: segment.to_owned(),
                });
            }
        }

        Ok(ProjectName(name.to_owned()))
    }

    /// Whether one path segment is on the charset. The walk that finds projects
    /// asks this of a directory name before it descends.
    pub fn is_segment(segment: &str) -> bool {
        let mut characters = segment.chars();

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

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The deepest segment — what the alternatives ladder suffixes, so that
    /// `acme/api` yields `acme/api-2` rather than a *nested* `acme/api/2`.
    pub fn last_segment(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or(&self.0)
    }

    /// This name with its last segment replaced, where the result is still a
    /// name.
    pub fn with_last_segment(&self, replacement: &str) -> Option<ProjectName> {
        let head = match self.0.rfind('/') {
            Some(slash) => &self.0[..=slash],
            None => "",
        };

        ProjectName::parse(&format!("{head}{replacement}")).ok()
    }

    /// Whether one of these two names would live **inside** the other's project
    /// directory, in either direction.
    ///
    /// Equal names are not nesting — that is `project_name_taken`, which has a
    /// remedy. This has none: the inner project would be swept away by a
    /// `project rm` of the outer one without being mentioned, and the walk,
    /// stopping at the first `project.toml`, would never have listed it
    /// (ADR-0009).
    pub fn nests_with(&self, other: &ProjectName) -> bool {
        inside(&self.0, &other.0) || inside(&other.0, &self.0)
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

/// Whether `inner` sits below `outer` — on a segment boundary, so `acme/apiary`
/// is not inside `acme/api`.
fn inside(inner: &str, outer: &str) -> bool {
    inner
        .strip_prefix(outer)
        .is_some_and(|rest| rest.starts_with('/'))
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
        for value in ["@", "@Codestyle", "@acme//codestyle", "@-api"] {
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
    fn the_sigil_and_not_the_charset_is_what_keeps_the_two_arms_apart() {
        let MemberRef::Project(name) = MemberRef::parse("@acme/codestyle").unwrap() else {
            panic!("a sigil makes it a project however many slashes follow");
        };
        assert_eq!(name.as_str(), "acme/codestyle");

        assert_eq!(
            MemberRef::parse("acme/codestyle").unwrap(),
            MemberRef::Repo(PathBuf::from("acme/codestyle")),
            "bare, the same text is still a relative path"
        );
    }

    #[test]
    fn a_name_is_a_path_of_segments_each_on_the_charset() {
        for name in [
            "codestyle",
            "acme/codestyle",
            "acme/platform/tooling/codestyle",
            "a.b/c_d-e/9",
        ] {
            assert_eq!(ProjectName::parse(name).unwrap().as_str(), name);
        }
    }

    #[test]
    fn every_way_a_path_could_escape_is_refused_by_the_charset() {
        for name in [
            "",
            "/acme",
            "acme/",
            "acme//api",
            ".",
            "..",
            "acme/../api",
            "acme/./api",
            "Acme/api",
            "acme/-api",
            "acme/.hidden",
        ] {
            assert!(
                matches!(
                    ProjectName::parse(name),
                    Err(EngineError::InvalidProjectName { .. })
                ),
                "{name:?} should not be a project name"
            );
        }
    }

    #[test]
    fn depth_is_uncapped_and_length_is_bounded_at_128() {
        let deep: String = std::iter::repeat_n("a", 60)
            .collect::<Vec<&str>>()
            .join("/");
        assert_eq!(deep.len(), 119);
        assert!(ProjectName::parse(&deep).is_ok(), "depth is not the bound");

        let long = "a".repeat(MAX_PROJECT_NAME_LENGTH);
        assert!(ProjectName::parse(&long).is_ok());
        assert!(ProjectName::parse(&format!("{long}a")).is_err());
    }

    #[test]
    fn the_two_directories_bitplane_puts_inside_a_project_are_reserved_as_segments() {
        for (name, segment) in [
            ("repo.git", "repo.git"),
            ("bin", "bin"),
            ("acme/bin", "bin"),
            ("acme/repo.git/api", "repo.git"),
        ] {
            assert_eq!(
                ProjectName::parse(name),
                Err(EngineError::ReservedNameSegment {
                    name: name.to_owned(),
                    segment: segment.to_owned(),
                }),
                "for {name}"
            );
        }

        assert!(
            ProjectName::parse("acme/binary").is_ok(),
            "the reservation is a whole segment, not a prefix"
        );
    }

    #[test]
    fn nesting_is_a_segment_boundary_in_either_direction_and_never_equality() {
        let parent = ProjectName::parse("acme").unwrap();
        let child = ProjectName::parse("acme/codestyle").unwrap();

        assert!(parent.nests_with(&child));
        assert!(child.nests_with(&parent), "and the other way round");
        assert!(!parent.nests_with(&parent), "equal is taken, not nesting");
        assert!(
            !ProjectName::parse("acme/api")
                .unwrap()
                .nests_with(&ProjectName::parse("acme/apiary").unwrap()),
            "a shared prefix that is not a whole segment is not nesting"
        );
    }

    #[test]
    fn the_ladder_suffixes_the_last_segment_and_never_the_whole_name() {
        assert_eq!(
            ProjectName::parse("acme/api")
                .unwrap()
                .with_last_segment("api-2")
                .unwrap()
                .as_str(),
            "acme/api-2"
        );
        assert_eq!(
            ProjectName::parse("api")
                .unwrap()
                .with_last_segment("api-2")
                .unwrap()
                .as_str(),
            "api-2"
        );
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
