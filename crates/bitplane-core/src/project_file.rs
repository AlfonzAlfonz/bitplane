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
use crate::scripts::{Script, ScriptCommand, ScriptName, ScriptPoint};
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

/// What a `[scripts.<name>]` table may hold: the two command forms, and a
/// toggle per point.
const LEGAL_SCRIPT_KEYS: [&str; 4] = [
    "argv",
    "shell",
    "post_worktree_create",
    "pre_worktree_remove",
];

/// A project, as the file holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectFile {
    pub version: i64,
    /// The integrity check. The directory name is the authority.
    pub name: ProjectName,
    pub source: ProjectSource,
    /// In **declaration order**, which is execution order. bitplane never
    /// rewrites this table, so the order cannot change behind the user's back.
    pub scripts: Vec<Script>,
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
    ///
    /// With no scripts: `add` and `adopt` write a file the user has not had a
    /// chance to declare any in, and bitplane never writes one on their behalf
    /// — which is the whole of the shell-alias trust posture.
    pub fn new(name: ProjectName, source: ProjectSource) -> ProjectFile {
        ProjectFile {
            version: CURRENT_VERSION,
            name,
            source,
            scripts: Vec::new(),
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
            scripts: read_scripts(path, &document)?,
        })
    }

    /// The scripts bound to `point`, in declaration order.
    ///
    /// The **resolved per-point sequence**, and the price of the toggle
    /// declaration form: *"what runs at `post_worktree_create`, in what order"*
    /// is not answerable from any single place in the file — you read every
    /// table, in order — so `bp project show` prints what this returns rather
    /// than leaving it as a config-reading problem (ADR-0007).
    pub fn at(&self, point: ScriptPoint) -> Vec<&Script> {
        self.scripts
            .iter()
            .filter(|script| script.points.contains(&point))
            .collect()
    }

    /// The script called `name`, where this project declares one.
    pub fn script(&self, name: &ScriptName) -> Option<&Script> {
        self.scripts.iter().find(|script| &script.name == name)
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
    /// back — which is also why **this does not render `[scripts]` at all**: a
    /// file this writes is a new one, and the user has declared none in it yet.
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

    /// The name this source would take if the user named none, and the segment
    /// that stopped it where there is none.
    ///
    /// **One rule, applied twice: use the namespace when there is one.** A
    /// source that names a host carries a forge path, which *is* a namespace —
    /// stable, globally unique, agreed on by everyone who clones the repo — so
    /// the whole path after the host becomes the name. A source that names no
    /// host is a filesystem path, which is an accident of where a home
    /// directory happens to sit, so only its last segment does (ADR-0009).
    ///
    /// The derived name is **lowercased silently**. That is a normalisation
    /// rather than a guess: two names differing only in case are one directory
    /// on a case-insensitive filesystem, so there is nothing here to choose
    /// between. It never reaches `--name`, which is the user's own words.
    pub fn default_name(&self) -> Result<ProjectName, EngineError> {
        let segments = self.name_segments();
        let lowercased: Vec<String> = segments
            .iter()
            .map(|segment| segment.to_ascii_lowercase())
            .collect();

        ProjectName::parse(&lowercased.join("/")).map_err(|why| EngineError::DerivedNameInvalid {
            derived: at_fault(&segments, &why),
        })
    }

    /// Whether two sources are the same repository, for the "you already have
    /// this" arm of `project_name_taken`.
    ///
    /// A url compared **as git would fetch it**: `…/codestyle.git` and
    /// `…/codestyle` are one remote, and somebody re-running an `add` with the
    /// suffix dropped has not asked for a second copy of it. An adopted path is
    /// compared as it is — bitplane canonicalises those on the way in.
    pub fn same_repo_as(&self, other: &ProjectSource) -> bool {
        match (self, other) {
            (ProjectSource::Owned { url: ours }, ProjectSource::Owned { url: theirs }) => {
                fetched_alike(ours) == fetched_alike(theirs)
            }
            (left, right) => left == right,
        }
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

    /// The segments a default name is built from: all of them behind a host,
    /// the last one where there is no host.
    fn name_segments(&self) -> Vec<&str> {
        let segments = self.segments();

        match self.names_a_host() {
            true => segments,
            false => segments.last().copied().into_iter().collect(),
        }
    }

    /// Whether the source spells a host — the scp-like `user@host:path`, or a
    /// `scheme://host/path` with a host in it.
    ///
    /// An **adopted** source never does; neither does a bare local path handed
    /// to `project add`, which is a git URL git will happily fetch from and a
    /// namespace nobody else shares.
    fn names_a_host(&self) -> bool {
        let ProjectSource::Owned { url } = self else {
            return false;
        };

        match url.split_once("://") {
            Some((_, after_scheme)) => !after_scheme.starts_with('/'),
            None => match (url.find(':'), url.find('/')) {
                (Some(colon), None) => colon > 0,
                (Some(colon), Some(slash)) => colon < slash && colon > 0,
                _ => false,
            },
        }
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

/// Reads `[scripts]`, in declaration order.
///
/// An absent table is no scripts, which is the ordinary case and not an error.
/// Everything else is checked here and nowhere later: a misspelled toggle in a
/// hand-written file must fail loudly at every read — `bp project show` and
/// `bp doctor` included — rather than silently costing the user a script that
/// never runs with no trace of why (ADR-0007).
fn read_scripts(
    path: &Path,
    document: &toml_edit::DocumentMut,
) -> Result<Vec<Script>, EngineError> {
    let Some(item) = document.get("scripts") else {
        return Ok(Vec::new());
    };
    let table = item
        .as_table_like()
        .ok_or_else(|| shaped_wrong(path, "scripts must be a table"))?;

    table
        .iter()
        .map(|(name, entry)| read_script(path, name, entry))
        .collect()
}

fn read_script(path: &Path, name: &str, entry: &toml_edit::Item) -> Result<Script, EngineError> {
    // Not a `script_error`: the legal-keys remedy answers a misspelled key, and
    // neither of these is one. A remedy that does not answer the failure it is
    // printed under is worse than none.
    let name = ScriptName::parse(name).map_err(|_| {
        shaped_wrong(
            path,
            format!(
                "{name:?} {}; use lowercase letters, digits and . _ -",
                crate::scripts::InvalidScriptName
            ),
        )
    })?;

    let table = entry
        .as_table_like()
        .ok_or_else(|| shaped_wrong(path, format!("[scripts.{name}] must be a table")))?;

    for (key, _) in table.iter() {
        if !LEGAL_SCRIPT_KEYS.contains(&key) {
            return Err(script_error(
                path,
                format!("unknown key {key:?} in [scripts.{name}]"),
            ));
        }
    }

    Ok(Script {
        command: read_command(path, &name, table)?,
        points: read_points(path, &name, table)?,
        name,
    })
}

/// `argv = [...]` **or** `shell = "..."`, and never both or neither.
fn read_command(
    path: &Path,
    name: &ScriptName,
    table: &dyn toml_edit::TableLike,
) -> Result<ScriptCommand, EngineError> {
    let argv = table.get("argv");
    let shell = table.get("shell");

    match (argv, shell) {
        (Some(_), Some(_)) => Err(script_error(
            path,
            format!("[scripts.{name}] has both argv and shell"),
        )),
        (None, None) => Err(script_error(
            path,
            format!("[scripts.{name}] has neither argv nor shell"),
        )),
        (Some(argv), None) => {
            let words = argv.as_array().ok_or_else(|| {
                script_error(path, format!("[scripts.{name}] argv must be an array"))
            })?;

            let argv: Option<Vec<String>> = words
                .iter()
                .map(|word| word.as_str().map(str::to_owned))
                .collect();
            let argv = argv.ok_or_else(|| {
                script_error(
                    path,
                    format!("[scripts.{name}] argv must be an array of strings"),
                )
            })?;

            if argv.is_empty() {
                return Err(script_error(
                    path,
                    format!("[scripts.{name}] argv names no command"),
                ));
            }

            Ok(ScriptCommand::Argv(argv))
        }
        (None, Some(shell)) => shell
            .as_str()
            .map(|line| ScriptCommand::Shell(line.to_owned()))
            .ok_or_else(|| script_error(path, format!("[scripts.{name}] shell must be a string"))),
    }
}

/// The points whose toggle is on, in the order [`ScriptPoint::ALL`] names them
/// — so two files that bind the same points agree however they spelled them.
fn read_points(
    path: &Path,
    name: &ScriptName,
    table: &dyn toml_edit::TableLike,
) -> Result<Vec<ScriptPoint>, EngineError> {
    let mut points = Vec::new();

    for point in ScriptPoint::ALL {
        let Some(toggle) = table.get(point.key()) else {
            continue;
        };
        let on = toggle.as_bool().ok_or_else(|| {
            script_error(
                path,
                format!("[scripts.{name}] {} must be true or false", point.key()),
            )
        })?;

        if on {
            points.push(point);
        }
    }

    Ok(points)
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

/// What a failed derivation names back to the user.
///
/// The **segment** at fault wherever there is one: the one off the charset, or
/// the one bitplane reserves. Where every segment was fine and the name was not
/// — a path over the length bound, or an empty source — the whole derived name,
/// because no single segment is the thing to look at.
///
/// Named **as the URL spelled it**, capitals and all, so it can be found in the
/// URL the user typed.
fn at_fault(segments: &[&str], why: &EngineError) -> String {
    let named = match why {
        EngineError::ReservedNameSegment { segment, .. } => segments
            .iter()
            .find(|candidate| candidate.to_ascii_lowercase() == *segment)
            .copied(),
        _ => segments
            .iter()
            .find(|segment| !ProjectName::is_segment(&segment.to_ascii_lowercase()))
            .copied(),
    };

    named
        .map(str::to_owned)
        .unwrap_or_else(|| segments.join("/"))
}

/// A url reduced to what git would fetch from: no trailing slashes, no trailing
/// `.git`.
fn fetched_alike(url: &str) -> &str {
    let trimmed = url.trim_end_matches('/');

    trimmed.strip_suffix(".git").unwrap_or(trimmed)
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

/// A `[scripts]` failure the legal-keys remedy would not answer.
fn shaped_wrong(path: &Path, message: impl Into<String>) -> EngineError {
    EngineError::ParseError {
        path: path.to_path_buf(),
        message: message.into(),
        legal_keys: Vec::new(),
    }
}

/// A `[scripts.<name>]` key, toggle or command form that is not one.
fn script_error(path: &Path, message: impl Into<String>) -> EngineError {
    EngineError::ParseError {
        path: path.to_path_buf(),
        message: message.into(),
        legal_keys: LEGAL_SCRIPT_KEYS
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
    fn scripts_are_read_in_declaration_order_with_their_points_and_their_commands() {
        let file = ProjectFile::parse(Path::new(PATH), &with_scripts()).unwrap();

        assert_eq!(
            file.scripts
                .iter()
                .map(|script| script.name.to_string())
                .collect::<Vec<String>>(),
            ["link-alfonz", "install", "stop-stack", "reset-db"],
            "declaration order is execution order, so it is what the file is read in"
        );
        assert_eq!(
            file.scripts[0].command,
            ScriptCommand::Argv(vec![
                "ln".to_owned(),
                "-s".to_owned(),
                "../../.alfonz".to_owned(),
                ".alfonz".to_owned(),
            ])
        );
        assert_eq!(
            file.scripts[1].command,
            ScriptCommand::Shell("pnpm i && pnpm build".to_owned())
        );
        assert_eq!(
            file.scripts[3].points,
            Vec::new(),
            "a script bound to no point is legal"
        );
    }

    #[test]
    fn the_resolved_per_point_sequence_is_declaration_order_within_each_point() {
        let file = ProjectFile::parse(Path::new(PATH), &with_scripts()).unwrap();

        assert_eq!(
            named(file.at(ScriptPoint::PostWorktreeCreate)),
            ["link-alfonz", "install"]
        );
        assert_eq!(
            named(file.at(ScriptPoint::PreWorktreeRemove)),
            ["stop-stack"]
        );
    }

    #[test]
    fn an_unknown_key_in_a_script_table_is_a_parse_error_listing_what_was_legal() {
        let text = format!(
            "{}\n[scripts.install]\nshell = \"pnpm i\"\npost_worktree_created = true\n",
            header()
        );

        let envelope = ProjectFile::parse(Path::new(PATH), &text)
            .unwrap_err()
            .envelope();

        assert_eq!(envelope.error, "parse_error");
        assert_eq!(
            envelope.message,
            format!("{PATH}: unknown key \"post_worktree_created\" in [scripts.install]")
        );
        assert_eq!(
            envelope.remedy.as_deref(),
            Some("Legal keys are argv, shell, post_worktree_create and pre_worktree_remove.")
        );
    }

    #[test]
    fn a_script_with_neither_command_form_or_with_both_is_a_parse_error() {
        let cases = [
            (
                "[scripts.install]\npost_worktree_create = true\n",
                "neither",
            ),
            (
                "[scripts.install]\nargv = [\"ln\"]\nshell = \"pnpm i\"\n",
                "both",
            ),
        ];

        for (table, expected) in cases {
            let error =
                ProjectFile::parse(Path::new(PATH), &format!("{}\n{table}", header())).unwrap_err();

            assert!(
                error.envelope().message.contains(expected),
                "for {table:?}: {}",
                error.envelope().message
            );
        }
    }

    #[test]
    fn a_point_toggle_that_is_not_a_boolean_is_a_parse_error_naming_it() {
        let text = format!(
            "{}\n[scripts.install]\nshell = \"pnpm i\"\npost_worktree_create = \"yes\"\n",
            header()
        );

        let error = ProjectFile::parse(Path::new(PATH), &text).unwrap_err();

        assert!(
            error
                .envelope()
                .message
                .contains("post_worktree_create must be true or false"),
            "got {}",
            error.envelope().message
        );
    }

    #[test]
    fn a_script_name_a_path_could_swallow_is_refused_before_it_reaches_one() {
        // The log is named after the script, so a name outside the plane-id
        // character set is never trusted into a path.
        let text = format!(
            "{}\n[scripts.\"../../etc/passwd\"]\nshell = \"true\"\n",
            header()
        );

        let error = ProjectFile::parse(Path::new(PATH), &text).unwrap_err();

        assert!(
            error
                .envelope()
                .message
                .contains("is not a valid script name"),
            "got {}",
            error.envelope().message
        );
    }

    #[test]
    fn rendering_is_for_a_new_file_only_and_drops_a_scripts_table() {
        // Pinned rather than fixed. A change to a file that already exists is a
        // `toml_edit` read-modify-write that touches `name` and `source` and
        // never `[scripts]` — which is what keeps declaration order and a
        // user's comments — so a verb that reaches for `render()` on a file it
        // read is reaching for the wrong tool, and this is where it finds out.
        let file = ProjectFile::parse(Path::new(PATH), &with_scripts()).unwrap();

        assert_eq!(file.scripts.len(), 4);
        assert!(!file.render().contains("scripts"), "got {}", file.render());
    }

    #[test]
    fn a_toggle_switched_off_binds_nothing() {
        let text = format!(
            "{}\n[scripts.install]\nshell = \"pnpm i\"\npost_worktree_create = false\n",
            header()
        );

        let file = ProjectFile::parse(Path::new(PATH), &text).unwrap();

        assert!(file.at(ScriptPoint::PostWorktreeCreate).is_empty());
    }

    fn header() -> String {
        concat!(
            "version = 1\nname = \"codestyle\"\n\n",
            "[source]\ntype = \"owned\"\nurl = \"git@host:acme/codestyle.git\"\n",
        )
        .to_owned()
    }

    fn with_scripts() -> String {
        format!(
            "{}\n{}",
            header(),
            concat!(
                "[scripts.link-alfonz]\n",
                "argv = [\"ln\", \"-s\", \"../../.alfonz\", \".alfonz\"]\n",
                "post_worktree_create = true\n\n",
                "[scripts.install]\n",
                "shell = \"pnpm i && pnpm build\"\n",
                "post_worktree_create = true\n\n",
                "[scripts.stop-stack]\n",
                "shell = \"docker compose down --remove-orphans\"\n",
                "pre_worktree_remove = true\n\n",
                "[scripts.reset-db]\n",
                "shell = \"pnpm db:reset && seed-fixtures\"\n",
            )
        )
    }

    fn named(scripts: Vec<&Script>) -> Vec<String> {
        scripts
            .iter()
            .map(|script| script.name.to_string())
            .collect()
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
    fn a_source_that_names_a_host_takes_the_whole_path_after_it() {
        let cases = [
            ("git@gitlab.com:acme/codestyle.git", "acme/codestyle"),
            (
                "git@gitlab.com:acme/platform/tooling/codestyle.git",
                "acme/platform/tooling/codestyle",
            ),
            ("https://github.com/acme/codestyle.git", "acme/codestyle"),
            ("ssh://git@gitlab.com/acme/api/", "acme/api"),
            ("git@gitlab.com:codestyle.git", "codestyle"),
        ];

        for (url, expected) in cases {
            let source = ProjectSource::Owned {
                url: url.to_owned(),
            };

            assert_eq!(
                source.default_name().unwrap().as_str(),
                expected,
                "for {url}"
            );
        }
    }

    #[test]
    fn a_source_that_names_no_host_keeps_its_last_segment() {
        // The same rule, not an exception to it: a forge path is a namespace
        // everyone who clones the repo agrees on, and a filesystem path is an
        // accident of where a home directory sits.
        let cases: [(ProjectSource, &str); 3] = [
            (
                ProjectSource::Adopted {
                    path: PathBuf::from("/Users/alfonz/projects/bitplane"),
                },
                "bitplane",
            ),
            (
                ProjectSource::Owned {
                    url: "/srv/git/acme/codestyle.git".to_owned(),
                },
                "codestyle",
            ),
            (
                ProjectSource::Owned {
                    url: "file:///srv/git/acme/codestyle.git".to_owned(),
                },
                "codestyle",
            ),
        ];

        for (source, expected) in cases {
            assert_eq!(
                source.default_name().unwrap().as_str(),
                expected,
                "for {source}"
            );
        }
    }

    #[test]
    fn a_derived_name_is_lowercased_and_nothing_else_is_guessed_at() {
        let source = ProjectSource::Owned {
            url: "git@gitlab.com:Acme/MyProject.git".to_owned(),
        };

        assert_eq!(source.default_name().unwrap().as_str(), "acme/myproject");
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
    fn a_derivation_that_fails_names_the_part_at_fault_and_offers_nothing() {
        let cases = [
            // The segment off the charset, whichever one it is.
            ("git@gitlab.com:acme/My Project.git", "My Project"),
            ("git@gitlab.com:acme/code+style.git", "code+style"),
            ("git@gitlab.com:-acme/codestyle.git", "-acme"),
            ("git@gitlab.com:acme/.hidden.git", ".hidden"),
            // The segment bitplane reserves, and not the last one.
            ("git@gitlab.com:acme/bin.git", "bin"),
            ("git@gitlab.com:acme/Bin/api.git", "Bin"),
            ("git@gitlab.com:acme/repo.git/api.git", "repo.git"),
        ];

        for (url, at_fault) in cases {
            let source = ProjectSource::Owned {
                url: url.to_owned(),
            };

            assert_eq!(
                source.default_name(),
                Err(EngineError::DerivedNameInvalid {
                    derived: at_fault.to_owned(),
                }),
                "for {url}, named as the URL spelled it"
            );
        }
    }

    #[test]
    fn a_derived_name_that_is_only_too_long_names_the_whole_name() {
        let deep: Vec<String> = (0..40).map(|ordinal| format!("group{ordinal}")).collect();
        let source = ProjectSource::Owned {
            url: format!("git@gitlab.com:{}.git", deep.join("/")),
        };

        assert_eq!(
            source.default_name(),
            Err(EngineError::DerivedNameInvalid {
                derived: deep.join("/"),
            }),
            "no one segment is the thing to look at, so none is blamed"
        );
    }

    #[test]
    fn a_url_and_the_same_url_without_its_git_suffix_are_one_repository() {
        let plain = ProjectSource::Owned {
            url: "git@gitlab.com:acme/codestyle".to_owned(),
        };

        assert!(plain.same_repo_as(&ProjectSource::Owned {
            url: "git@gitlab.com:acme/codestyle.git".to_owned(),
        }));
        assert!(plain.same_repo_as(&ProjectSource::Owned {
            url: "git@gitlab.com:acme/codestyle/".to_owned(),
        }));
        assert!(
            !plain.same_repo_as(&ProjectSource::Owned {
                url: "git@github.com:acme/codestyle.git".to_owned(),
            }),
            "the host is not part of the name, and is part of the repository"
        );
        assert!(!plain.same_repo_as(&ProjectSource::Adopted {
            path: PathBuf::from("/Users/alfonz/projects/codestyle"),
        }));
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
