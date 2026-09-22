//! `project.toml`: a version, a name, a source, and the scripts (ADR-0007).
//!
//! ```toml
//! version = 1
//! name = "codestyle"
//!
//! [source]
//! type = "owned"
//! url = "git@gitlab.com:acme/codestyle.git"
//! ```
//!
//! **`name` is an integrity check only.** The directory name *is* the name, and
//! on conflict the directory wins; the field exists so a project directory
//! moved or renamed outside bitplane is detectable, and so an interrupted
//! `project rename` is distinguishable from a completed one.
//!
//! The kind is named after **ownership**, not after the payload, because every
//! behavioural difference in the design branches on ownership: the plane-branch
//! invariant, `push.default = current`, `destroy` deleting the member's branch,
//! occupying no branch, and having something to fetch. The 1:1 correspondence
//! — an owned source is always a `url`, an adopted source always a `path` — is
//! a rule rather than a coincidence, so a `type` that disagrees with its
//! payload is a [`EngineError::ParseError`].
//!
//! `fetched_at` is **not** a field: it is the mtime of the source repo's
//! `FETCH_HEAD`, which git writes on every fetch. Storing it would make every
//! fetch do a read-modify-write under the project lock for a field nothing
//! gates on. `default_branch` is not a field either — git already stores it,
//! durably, at `refs/remotes/origin/HEAD` (ADR-0005).

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::EngineError;
use crate::member::{MalformedWorktreePath, ProjectName, WorktreePath};
use crate::toml_text::quoted;

/// The schema version every project file carries, from the first commit.
pub const CURRENT_VERSION: i64 = 1;

/// The name of the file, at the root of the project directory. A project is
/// exactly a directory containing one.
pub const PROJECT_FILE_NAME: &str = "project.toml";

/// The four keys, and nothing else.
const LEGAL_KEYS: [&str; 4] = ["version", "name", "source", "scripts"];

/// The two keys a `[source]` table may hold beside its `type`.
const LEGAL_SOURCE_KEYS: [&str; 3] = ["type", "url", "path"];

/// A project, as the file holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectFile {
    pub version: i64,
    /// The integrity check. The directory name is the authority.
    pub name: ProjectName,
    pub source: ProjectSource,
}

/// Where a project's repo comes from — and, because the two are in 1:1
/// correspondence, which kind of project it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProjectSource {
    /// bitplane built the source repo and owns its refs and its config.
    Owned { url: String },
    /// The user's own checkout, pointed at in place. bitplane writes nothing
    /// into it.
    Adopted { path: PathBuf },
}

impl ProjectFile {
    /// A new project's file, at the current version.
    pub fn new(name: ProjectName, source: ProjectSource) -> ProjectFile {
        ProjectFile {
            version: CURRENT_VERSION,
            name,
            source,
        }
    }

    /// Reads `text`, which came from `path`.
    ///
    /// `path` is carried only so a failure names the file; nothing about the
    /// parse depends on it.
    pub fn parse(path: &Path, text: &str) -> Result<ProjectFile, EngineError> {
        let document: toml_edit::DocumentMut = text
            .parse()
            .map_err(|err: toml_edit::TomlError| parse_error(path, err.message()))?;

        for (key, _) in document.iter() {
            if !LEGAL_KEYS.contains(&key) {
                return Err(parse_error(path, format!("unknown key {key:?}")));
            }
        }

        Ok(ProjectFile {
            version: read_version(path, &document)?,
            name: read_name(path, &document)?,
            source: read_source(path, &document)?,
        })
    }

    /// Reads the project file at `path`.
    pub fn read(path: &Path) -> Result<ProjectFile, EngineError> {
        let text = std::fs::read_to_string(path).map_err(|err| EngineError::io(path, err))?;
        ProjectFile::parse(path, &text)
    }

    /// The whole text of a **new** project file.
    ///
    /// Rendered rather than round-tripped, because `add` and `adopt` write a
    /// file that did not exist. Changing one that does is a `toml_edit`
    /// read-modify-write that touches `name` and `source` and never
    /// `[scripts]`, so a user's declaration order cannot change behind their
    /// back.
    pub fn render(&self) -> String {
        let (kind, key, value) = self.source.spelling();

        format!(
            "version = {}\nname = {}\n\n[source]\ntype = {}\n{key} = {}\n",
            self.version,
            quoted(self.name.as_str()),
            quoted(kind),
            quoted(&value),
        )
    }
}

impl ProjectSource {
    /// Where a worktree of this source lands, relative to the plane directory
    /// root: the last two segments of the source's path, per the layout table
    /// in `CONTEXT.md`.
    ///
    /// Derived here rather than by [`crate::MemberRef`] because a URL is not a
    /// path — `git@gitlab.com:acme/codestyle.git` has to lose its transport
    /// and its `.git` before `acme/codestyle` is visible.
    pub fn worktree_path(&self) -> Result<WorktreePath, EngineError> {
        let segments = self.segments();
        let tail: Vec<&str> = segments.iter().rev().take(2).rev().copied().collect();

        WorktreePath::parse(&tail.join("/")).map_err(|why| match why {
            MalformedWorktreePath::ReservedSegment => EngineError::ReservedPathSegment {
                path: tail.join("/"),
            },
            _ => EngineError::InvalidRequest {
                message: format!("{self} has no name to derive a worktree path from"),
            },
        })
    }

    /// The name this source would take if the user named none: the last
    /// segment of its path.
    pub fn default_name(&self) -> Result<ProjectName, EngineError> {
        let segments = self.segments();

        ProjectName::parse(segments.last().copied().unwrap_or_default())
    }

    /// The last segment of the source's path, whether or not it is a project
    /// name. What a failed derivation names back to the user.
    pub fn last_segment(&self) -> String {
        self.segments()
            .last()
            .copied()
            .unwrap_or_default()
            .to_owned()
    }

    /// The nearest project name to [`ProjectSource::default_name`]: lowercased,
    /// with everything outside the charset folded to `-`.
    ///
    /// **Offered for the user to type, never applied on their behalf** — a
    /// forge that names a repo `MyProject` has not told bitplane what the
    /// project should be called, and guessing is the silent disambiguation the
    /// design refuses everywhere else.
    pub fn suggested_name(&self) -> Option<ProjectName> {
        let folded: String = self
            .last_segment()
            .chars()
            .map(|character| match character {
                'A'..='Z' => character.to_ascii_lowercase(),
                'a'..='z' | '0'..='9' | '.' | '_' | '-' => character,
                _ => '-',
            })
            .collect();

        let tidied = folded
            .split('-')
            .filter(|run| !run.is_empty())
            .collect::<Vec<&str>>()
            .join("-");

        ProjectName::parse(tidied.trim_start_matches(['.', '_'])).ok()
    }

    /// The name to suggest when [`ProjectSource::default_name`] is taken —
    /// `acme-codestyle` for `acme/codestyle`. Absent where there is no segment
    /// before the last, or where joining the two is not a project name.
    pub fn qualified_name(&self) -> Option<ProjectName> {
        let segments = self.segments();
        let [.., parent, last] = segments.as_slice() else {
            return None;
        };

        ProjectName::parse(&format!("{parent}-{last}")).ok()
    }

    /// Whether bitplane built this source repo, and therefore owns its refs and
    /// its config.
    pub fn is_owned(&self) -> bool {
        matches!(self, ProjectSource::Owned { .. })
    }

    /// The word bitplane prints for this kind, and stores as the `type`.
    pub fn kind(&self) -> &'static str {
        match self {
            ProjectSource::Owned { .. } => "owned",
            ProjectSource::Adopted { .. } => "adopted",
        }
    }

    /// The path part of the source, split into segments, with the transport and
    /// any trailing `.git` gone.
    fn segments(&self) -> Vec<&str> {
        let path = match self {
            ProjectSource::Owned { url } => url_path(url),
            ProjectSource::Adopted { path } => path.to_str().unwrap_or_default(),
        };

        path_segments(path)
    }

    /// The `type`, and the one payload key that goes with it.
    fn spelling(&self) -> (&'static str, &'static str, String) {
        match self {
            ProjectSource::Owned { url } => ("owned", "url", url.clone()),
            ProjectSource::Adopted { path } => ("adopted", "path", path.display().to_string()),
        }
    }
}

/// Rendered as what the user typed: the URL for an owned project, the path for
/// an adopted one.
impl fmt::Display for ProjectSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProjectSource::Owned { url } => f.write_str(url),
            ProjectSource::Adopted { path } => f.write_str(&path.display().to_string()),
        }
    }
}

fn read_version(path: &Path, document: &toml_edit::DocumentMut) -> Result<i64, EngineError> {
    let version = document
        .get("version")
        .and_then(toml_edit::Item::as_integer)
        .ok_or_else(|| parse_error(path, "version must be an integer"))?;

    if version != CURRENT_VERSION {
        return Err(parse_error(
            path,
            format!("version {version} is not a version bitplane understands"),
        ));
    }

    Ok(version)
}

fn read_name(path: &Path, document: &toml_edit::DocumentMut) -> Result<ProjectName, EngineError> {
    let name = document
        .get("name")
        .and_then(toml_edit::Item::as_str)
        .ok_or_else(|| parse_error(path, "name must be a string"))?;

    ProjectName::parse(name)
        .map_err(|_| parse_error(path, format!("{name:?} is not a valid project name")))
}

/// Reads `[source]`, holding the 1:1 rule: an owned source is always a `url`,
/// an adopted source always a `path`.
fn read_source(
    path: &Path,
    document: &toml_edit::DocumentMut,
) -> Result<ProjectSource, EngineError> {
    let table = document
        .get("source")
        .and_then(toml_edit::Item::as_table_like)
        .ok_or_else(|| parse_error(path, "source must be a table"))?;

    for (key, _) in table.iter() {
        if !LEGAL_SOURCE_KEYS.contains(&key) {
            return Err(source_error(
                path,
                format!("unknown key {key:?} in [source]"),
            ));
        }
    }

    let kind = table
        .get("type")
        .and_then(toml_edit::Item::as_str)
        .ok_or_else(|| source_error(path, "source type must be a string"))?;
    let url = table.get("url").and_then(toml_edit::Item::as_str);
    let local = table.get("path").and_then(toml_edit::Item::as_str);

    match (kind, url, local) {
        ("owned", Some(url), None) => Ok(ProjectSource::Owned {
            url: url.to_owned(),
        }),
        ("adopted", None, Some(local)) => Ok(ProjectSource::Adopted {
            path: PathBuf::from(local),
        }),
        ("owned", _, _) => Err(source_error(path, "an owned source is always a url")),
        ("adopted", _, _) => Err(source_error(path, "an adopted source is always a path")),
        (other, _, _) => Err(source_error(
            path,
            format!("{other:?} is not a project kind"),
        )),
    }
}

/// The path part of a git URL, whatever transport spells it.
///
/// Three shapes, and git accepts all of them: `scheme://host/path`,
/// the scp-like `user@host:path`, and a bare local path. The scp-like form is
/// told apart by its colon coming **before** any slash, which is what stops
/// `/srv/git/odd:name` being read as a host.
fn url_path(url: &str) -> &str {
    if let Some((_, after_scheme)) = url.split_once("://") {
        return after_scheme
            .split_once('/')
            .map(|(_, path)| path)
            .unwrap_or("");
    }

    match (url.find(':'), url.find('/')) {
        (Some(colon), None) => &url[colon + 1..],
        (Some(colon), Some(slash)) if colon < slash => &url[colon + 1..],
        _ => url,
    }
}

/// The segments of a source path, with any trailing slashes and `.git` gone.
fn path_segments(path: &str) -> Vec<&str> {
    path.trim_end_matches('/')
        .strip_suffix(".git")
        .unwrap_or(path.trim_end_matches('/'))
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect()
}

fn parse_error(path: &Path, message: impl Into<String>) -> EngineError {
    EngineError::ParseError {
        path: path.to_path_buf(),
        message: message.into(),
        legal_keys: LEGAL_KEYS.iter().map(|key| (*key).to_owned()).collect(),
    }
}

fn source_error(path: &Path, message: impl Into<String>) -> EngineError {
    EngineError::ParseError {
        path: path.to_path_buf(),
        message: message.into(),
        legal_keys: LEGAL_SOURCE_KEYS
            .iter()
            .map(|key| (*key).to_owned())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: &str = "/projects/codestyle/project.toml";

    #[test]
    fn an_owned_project_is_a_version_a_name_and_a_url() {
        let file = owned();

        assert_eq!(
            file.render(),
            concat!(
                "version = 1\n",
                "name = \"codestyle\"\n",
                "\n",
                "[source]\n",
                "type = \"owned\"\n",
                "url = \"git@gitlab.com:acme/codestyle.git\"\n",
            )
        );
    }

    #[test]
    fn what_is_rendered_parses_back_to_what_was_rendered() {
        for file in [owned(), adopted()] {
            assert_eq!(
                ProjectFile::parse(Path::new(PATH), &file.render()).unwrap(),
                file
            );
        }
    }

    #[test]
    fn an_unrecognised_top_level_key_is_a_parse_error_naming_it() {
        let text = concat!(
            "version = 1\nname = \"codestyle\"\ndefault_branch = \"main\"\n\n",
            "[source]\ntype = \"owned\"\nurl = \"git@host:acme/codestyle.git\"\n",
        );

        let error = ProjectFile::parse(Path::new(PATH), text).unwrap_err();

        let envelope = error.envelope();
        assert_eq!(envelope.error, "parse_error");
        assert!(
            envelope.message.contains("default_branch"),
            "got {}",
            envelope.message
        );
        assert_eq!(
            envelope.remedy.as_deref(),
            Some("Legal keys are version, name, source and scripts.")
        );
    }

    #[test]
    fn a_type_that_disagrees_with_its_payload_is_a_parse_error() {
        let cases = [
            ("owned", "path = \"/Users/alfonz/projects/codestyle\""),
            ("adopted", "url = \"git@host:acme/codestyle.git\""),
        ];

        for (kind, payload) in cases {
            let text = format!(
                "version = 1\nname = \"codestyle\"\n\n[source]\ntype = {kind:?}\n{payload}\n"
            );

            let error = ProjectFile::parse(Path::new(PATH), &text).unwrap_err();
            assert!(
                error.envelope().message.contains("always a"),
                "for {kind}: {}",
                error.envelope().message
            );
        }
    }

    #[test]
    fn a_kind_bitplane_does_not_know_is_a_parse_error_naming_it() {
        let text = concat!(
            "version = 1\nname = \"codestyle\"\n\n",
            "[source]\ntype = \"cloned\"\nurl = \"git@host:acme/codestyle.git\"\n",
        );

        let error = ProjectFile::parse(Path::new(PATH), text).unwrap_err();

        assert!(
            error.envelope().message.contains("cloned"),
            "got {}",
            error.envelope().message
        );
    }

    #[test]
    fn a_scripts_table_is_legal_and_left_alone() {
        // Scripts arrive with `bp run`; refusing a file that declares them
        // would refuse a file this schema says is valid.
        let text = concat!(
            "version = 1\nname = \"codestyle\"\n\n",
            "[source]\ntype = \"owned\"\nurl = \"git@host:acme/codestyle.git\"\n\n",
            "[scripts.install]\nshell = \"pnpm i\"\npost_worktree_create = true\n",
        );

        assert_eq!(
            ProjectFile::parse(Path::new(PATH), text).unwrap().name,
            ProjectName::parse("codestyle").unwrap()
        );
    }

    #[test]
    fn a_version_bitplane_does_not_understand_is_refused_rather_than_guessed_at() {
        let text = concat!(
            "version = 2\nname = \"codestyle\"\n\n",
            "[source]\ntype = \"owned\"\nurl = \"git@host:acme/codestyle.git\"\n",
        );

        let error = ProjectFile::parse(Path::new(PATH), text).unwrap_err();

        assert!(
            error.envelope().message.contains("version 2"),
            "got {}",
            error.envelope().message
        );
    }

    #[test]
    fn the_worktree_path_is_the_last_two_segments_whatever_transport_spelled_it() {
        let cases = [
            ("git@gitlab.com:acme/codestyle.git", "acme/codestyle"),
            ("https://gitlab.com/acme/codestyle.git", "acme/codestyle"),
            ("https://gitlab.com/acme/codestyle", "acme/codestyle"),
            ("ssh://git@gitlab.com/acme/codestyle.git", "acme/codestyle"),
            ("ssh://git@gitlab.com:2222/acme/codestyle", "acme/codestyle"),
            ("file:///srv/git/acme/codestyle.git", "acme/codestyle"),
            ("/srv/git/acme/codestyle.git", "acme/codestyle"),
            ("git@gitlab.com:codestyle.git", "codestyle"),
            (
                "https://gitlab.com/acme/team/codestyle.git",
                "team/codestyle",
            ),
            ("git@gitlab.com:acme/codestyle.git/", "acme/codestyle"),
        ];

        for (url, expected) in cases {
            let source = ProjectSource::Owned {
                url: url.to_owned(),
            };
            assert_eq!(
                source.worktree_path().unwrap().to_string(),
                expected,
                "for {url}"
            );
        }
    }

    #[test]
    fn an_adopted_source_derives_its_path_from_the_checkout() {
        let source = ProjectSource::Adopted {
            path: PathBuf::from("/Users/alfonz/projects/bitplane"),
        };

        assert_eq!(
            source.worktree_path().unwrap().to_string(),
            "projects/bitplane"
        );
    }

    #[test]
    fn a_source_landing_in_the_reserved_segment_is_refused() {
        let source = ProjectSource::Owned {
            url: "git@gitlab.com:.bitplane/tools.git".to_owned(),
        };

        let error = source.worktree_path().unwrap_err();

        assert!(
            matches!(error, EngineError::ReservedPathSegment { .. }),
            "got {error:?}"
        );
        assert_eq!(
            error.to_string(),
            "a worktree of this repo would land at .bitplane/tools, which is reserved"
        );
    }

    #[test]
    fn the_default_name_is_the_last_segment_and_the_suggestion_qualifies_it() {
        let source = ProjectSource::Owned {
            url: "git@gitlab.com:acme/codestyle.git".to_owned(),
        };

        assert_eq!(source.default_name().unwrap().as_str(), "codestyle");
        assert_eq!(
            source.qualified_name().map(|name| name.to_string()),
            Some("acme-codestyle".to_owned())
        );
    }

    #[test]
    fn a_name_the_charset_will_not_take_is_folded_into_one_to_suggest() {
        let cases = [
            ("git@gitlab.com:acme/MyProject.git", Some("myproject")),
            ("git@gitlab.com:acme/My Project.git", Some("my-project")),
            ("git@gitlab.com:acme/.hidden.git", Some("hidden")),
            ("git@gitlab.com:acme/a--b.git", Some("a-b")),
            ("git@gitlab.com:acme/v1.0.git", Some("v1.0")),
            ("git@gitlab.com:acme/---.git", None),
        ];

        for (url, expected) in cases {
            let source = ProjectSource::Owned {
                url: url.to_owned(),
            };

            assert_eq!(
                source.suggested_name().map(|name| name.to_string()),
                expected.map(str::to_owned),
                "for {url}"
            );
        }
    }

    #[test]
    fn a_name_a_path_cannot_hold_is_rendered_as_toml_rather_than_as_rust_debug() {
        // A decomposed accent is what macOS stores for `é` in a filename, and
        // `{:?}` spells it with an escape TOML does not have.
        let file = ProjectFile::new(
            ProjectName::parse("eclair").unwrap(),
            ProjectSource::Adopted {
                path: PathBuf::from("/Users/alfonz/projects/e\u{301}clair"),
            },
        );

        assert_eq!(
            ProjectFile::parse(Path::new(PATH), &file.render()).unwrap(),
            file,
            "rendered as: {}",
            file.render()
        );
    }

    #[test]
    fn a_source_whose_last_segment_is_not_a_project_name_has_no_default() {
        let source = ProjectSource::Owned {
            url: "git@gitlab.com:acme/Codestyle.git".to_owned(),
        };

        assert!(
            matches!(
                source.default_name(),
                Err(EngineError::InvalidProjectName { .. })
            ),
            "an uppercase name is the user's to supply with --name"
        );
    }

    #[test]
    fn a_source_with_one_segment_has_nothing_to_qualify_it_with() {
        let source = ProjectSource::Owned {
            url: "git@gitlab.com:codestyle.git".to_owned(),
        };

        assert_eq!(source.qualified_name(), None);
    }

    #[test]
    fn a_source_round_trips_over_the_wire_tagged_by_its_kind() {
        for source in [
            ProjectSource::Owned {
                url: "git@gitlab.com:acme/codestyle.git".to_owned(),
            },
            ProjectSource::Adopted {
                path: PathBuf::from("/Users/alfonz/projects/bitplane"),
            },
        ] {
            let json = serde_json::to_string(&source).unwrap();

            assert!(
                json.contains(&format!(r#""type":"{}""#, source.kind())),
                "got {json}"
            );
            assert_eq!(
                serde_json::from_str::<ProjectSource>(&json).unwrap(),
                source
            );
        }
    }

    fn owned() -> ProjectFile {
        ProjectFile::new(
            ProjectName::parse("codestyle").unwrap(),
            ProjectSource::Owned {
                url: "git@gitlab.com:acme/codestyle.git".to_owned(),
            },
        )
    }

    fn adopted() -> ProjectFile {
        ProjectFile::new(
            ProjectName::parse("bitplane").unwrap(),
            ProjectSource::Adopted {
                path: PathBuf::from("/Users/alfonz/projects/bitplane"),
            },
        )
    }
}
