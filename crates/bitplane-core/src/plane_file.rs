//! `plane.toml`: three keys, and a path-keyed membership table (ADR-0008).
//!
//! ```toml
//! version = 1
//! id = "bp-a3f9c2e1"
//!
//! [members]
//! "acme/codestyle"    = "@codestyle"
//! "projects/bitplane" = "/Users/alfonz/projects/bitplane"
//! ```
//!
//! It is the **membership list**, not the desired state: which worktrees are in
//! this plane, and what each is a worktree of. It carries no branch, no status,
//! no host and no timestamp — every other candidate was either already stored by
//! git or the filesystem, or would have made a read write.
//!
//! The key is the worktree's path relative to the plane directory root; the
//! value names the source. Keys are rejected if absolute, containing `..`,
//! starting with `./`, or beginning with `.bitplane`, because a `..` in a
//! hand-edited file turns a later per-member removal into a write *outside* the
//! plane directory.
//!
//! Hand-editing is tolerated but not documented, which is why this reads and
//! writes through `toml_edit` rather than serde round-tripping: a user's
//! `# do not reap, long-running migration` must survive the next write.

use std::fmt;
use std::path::Path;

use toml_edit::{DocumentMut, Item, TableLike, Value};

use crate::error::EngineError;
use crate::member::{MemberRef, WorktreePath};
use crate::plane_id::PlaneId;
use crate::toml_text::quoted;

/// The schema version every plane file carries, from the first commit.
pub const CURRENT_VERSION: i64 = 1;

/// The name of the file, at the root of the plane directory.
pub const PLANE_FILE_NAME: &str = "plane.toml";

/// The three keys, and nothing else.
const LEGAL_KEYS: [&str; 3] = ["version", "id", "members"];

/// A plane's membership, as the file holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaneFile {
    pub version: i64,
    pub id: PlaneId,
    /// In declaration order, which is the order every fan-out prints.
    pub members: Vec<Member>,
}

/// One worktree, and what it is a worktree of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub path: WorktreePath,
    pub source: MemberRef,
}

impl PlaneFile {
    /// A new plane's membership, at the current version.
    pub fn new(id: PlaneId, members: Vec<Member>) -> PlaneFile {
        PlaneFile {
            version: CURRENT_VERSION,
            id,
            members,
        }
    }

    /// Reads `text`, which came from `path`.
    ///
    /// `path` is carried only so a failure names the file; nothing about the
    /// parse depends on it.
    pub fn parse(path: &Path, text: &str) -> Result<PlaneFile, EngineError> {
        let document: DocumentMut = text
            .parse()
            .map_err(|err: toml_edit::TomlError| parse_error(path, err.message()))?;

        for (key, _) in document.iter() {
            if !LEGAL_KEYS.contains(&key) {
                return Err(parse_error(path, format!("unknown key {key:?}")));
            }
        }

        let version = read_version(path, &document)?;
        let id = read_id(path, &document)?;
        let members = read_members(path, &document)?;

        Ok(PlaneFile {
            version,
            id,
            members,
        })
    }

    /// Reads the plane file at `path`.
    pub fn read(path: &Path) -> Result<PlaneFile, EngineError> {
        let text = std::fs::read_to_string(path).map_err(|err| EngineError::io(path, err))?;
        PlaneFile::parse(path, &text)
    }

    /// The whole text of a **new** plane file.
    ///
    /// Rendered rather than round-tripped, because `create` writes a file that
    /// did not exist. Changing the membership of one that does is a
    /// `toml_edit` read-modify-write, and arrives with `bp add`.
    pub fn render(&self) -> String {
        let keys: Vec<String> = self
            .members
            .iter()
            .map(|member| quoted(&member.path.to_string()))
            .collect();
        let width = keys.iter().map(String::len).max().unwrap_or(0);

        let mut text = format!(
            "version = {}\nid = {}\n\n[members]\n",
            self.version,
            quoted(self.id.as_str())
        );

        for (member, key) in self.members.iter().zip(&keys) {
            // Aligned on the `=`, so the file stays worth reading by hand —
            // which is the whole reason the state is plain text. Measured on
            // the **quoted** key, because that is what lands in the file.
            let padding = " ".repeat(width - key.len());
            text.push_str(&format!(
                "{key}{padding} = {}\n",
                quoted(&member.source.to_string())
            ));
        }

        text
    }
}

impl fmt::Display for PlaneFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

fn read_version(path: &Path, document: &DocumentMut) -> Result<i64, EngineError> {
    let version = document
        .get("version")
        .and_then(Item::as_integer)
        .ok_or_else(|| parse_error(path, "version must be an integer"))?;

    if version != CURRENT_VERSION {
        return Err(parse_error(
            path,
            format!("version {version} is not a version bitplane understands"),
        ));
    }

    Ok(version)
}

fn read_id(path: &Path, document: &DocumentMut) -> Result<PlaneId, EngineError> {
    let id = document
        .get("id")
        .and_then(Item::as_str)
        .ok_or_else(|| parse_error(path, "id must be a string"))?;

    PlaneId::parse(id).map_err(|_| parse_error(path, format!("{id:?} is not a valid plane id")))
}

fn read_members(path: &Path, document: &DocumentMut) -> Result<Vec<Member>, EngineError> {
    let Some(item) = document.get("members") else {
        return Err(parse_error(path, "members is missing"));
    };
    // `as_table_like`, not `as_table`: a hand-edited file may spell the
    // membership inline (`members = { "acme/api" = "@api" }`), and refusing
    // that would be refusing valid TOML.
    let table: &dyn TableLike = item
        .as_table_like()
        .ok_or_else(|| parse_error(path, "members must be a table"))?;

    let mut members: Vec<Member> = Vec::new();

    for (key, value) in table.iter() {
        // A nested table is the failure a dotted key produces silently, which
        // is why member keys are quoted (ADR-0008).
        let Some(Value::String(source)) = value.as_value() else {
            return Err(parse_error(
                path,
                format!("member {key:?} must be a string naming what it is a worktree of"),
            ));
        };

        let worktree = WorktreePath::parse(key)
            .map_err(|why| parse_error(path, format!("member key {key:?} {}", why.reason())))?;
        let source = MemberRef::parse(source.value()).map_err(|_| {
            parse_error(
                path,
                format!(
                    "member {key:?} names {:?}, which is neither a project nor a path",
                    source.value()
                ),
            )
        })?;

        if let Some(earlier) = members.iter().find(|member| member.source == source) {
            return Err(parse_error(
                path,
                format!(
                    "members {:?} and {key:?} are both worktrees of {source}",
                    earlier.path.to_string()
                ),
            ));
        }

        members.push(Member {
            path: worktree,
            source,
        });
    }

    Ok(members)
}

fn parse_error(path: &Path, message: impl Into<String>) -> EngineError {
    EngineError::ParseError {
        path: path.to_path_buf(),
        message: message.into(),
        legal_keys: LEGAL_KEYS.iter().map(|key| (*key).to_owned()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const PATH: &str = "/planes/bp-a3f9c2e1/plane.toml";

    #[test]
    fn the_file_is_exactly_three_keys_and_a_path_keyed_table() {
        let file = plane_file();

        assert_eq!(
            file.render(),
            concat!(
                "version = 1\n",
                "id = \"bp-a3f9c2e1\"\n",
                "\n",
                "[members]\n",
                "\"acme/codestyle\"    = \"@codestyle\"\n",
                "\"projects/bitplane\" = \"/Users/alfonz/projects/bitplane\"\n",
            )
        );
    }

    #[test]
    fn what_is_rendered_parses_back_to_what_was_rendered() {
        let file = plane_file();

        assert_eq!(
            PlaneFile::parse(Path::new(PATH), &file.render()).unwrap(),
            file
        );
    }

    #[test]
    fn a_member_path_a_filesystem_allows_is_rendered_as_toml_rather_than_rust_debug() {
        // macOS stores `é` decomposed, so this is an ordinary path rather than
        // an exotic one — and `{:?}` spells the combining accent with an escape
        // TOML does not have, producing a file bitplane could never read back.
        let file = PlaneFile::new(
            PlaneId::parse("bp-a3f9c2e1").unwrap(),
            vec![Member {
                path: WorktreePath::parse("projects/e\u{301}clair").unwrap(),
                source: MemberRef::Repo(PathBuf::from("/Users/alfonz/projects/e\u{301}clair")),
            }],
        );

        assert_eq!(
            PlaneFile::parse(Path::new(PATH), &file.render()).unwrap(),
            file,
            "rendered as: {}",
            file.render()
        );
    }

    #[test]
    fn membership_keeps_the_order_it_was_written_in() {
        let text = concat!(
            "version = 1\nid = \"bp-a3f9c2e1\"\n\n[members]\n",
            "\"z/last\" = \"/repos/last\"\n",
            "\"a/first\" = \"/repos/first\"\n",
        );

        let file = PlaneFile::parse(Path::new(PATH), text).unwrap();

        let keys: Vec<String> = file
            .members
            .iter()
            .map(|member| member.path.to_string())
            .collect();
        assert_eq!(
            keys,
            ["z/last", "a/first"],
            "order is what a fan-out prints"
        );
    }

    #[test]
    fn an_unrecognised_top_level_key_is_a_parse_error_naming_it() {
        let text = "version = 1\nid = \"bp-a3f9c2e1\"\nhost = \"local\"\n\n[members]\n";

        let error = PlaneFile::parse(Path::new(PATH), text).unwrap_err();

        let envelope = error.envelope();
        assert_eq!(envelope.error, "parse_error");
        assert!(
            envelope.message.contains("host"),
            "got {}",
            envelope.message
        );
        assert!(envelope.message.contains(PATH), "got {}", envelope.message);
        assert_eq!(
            envelope.remedy.as_deref(),
            Some("Legal keys are version, id and members.")
        );
    }

    #[test]
    fn every_dangerous_member_key_is_a_parse_error_naming_the_key() {
        let cases = [
            ("/acme/codestyle", "is absolute"),
            ("../outside", "contains .."),
            ("./acme", "starts with ./"),
            (".bitplane/api", "begins with .bitplane"),
        ];

        for (key, reason) in cases {
            let text = format!(
                "version = 1\nid = \"bp-a3f9c2e1\"\n\n[members]\n{key:?} = \"/repos/api\"\n"
            );

            let error = PlaneFile::parse(Path::new(PATH), &text).unwrap_err();
            let message = error.envelope().message;

            assert!(message.contains(key), "{key:?} unnamed in: {message}");
            assert!(
                message.contains(reason),
                "{key:?} unexplained in: {message}"
            );
        }
    }

    #[test]
    fn membership_spelled_inline_is_still_membership() {
        let text = concat!(
            "version = 1\nid = \"bp-a3f9c2e1\"\n",
            "members = { \"acme/codestyle\" = \"@codestyle\" }\n",
        );

        let file = PlaneFile::parse(Path::new(PATH), text).unwrap();

        assert_eq!(file.members.len(), 1);
        assert_eq!(file.members[0].source.to_string(), "@codestyle");
    }

    #[test]
    fn a_nested_table_under_members_is_a_parse_error() {
        let text = concat!(
            "version = 1\nid = \"bp-a3f9c2e1\"\n\n",
            "[members]\n[members.\"acme/codestyle\"]\nproject = \"codestyle\"\n"
        );

        let error = PlaneFile::parse(Path::new(PATH), text).unwrap_err();

        assert!(
            error.envelope().message.contains("acme/codestyle"),
            "got {}",
            error.envelope().message
        );
    }

    #[test]
    fn a_dotted_key_is_caught_as_the_nested_table_it_silently_becomes() {
        // `foo.bar = "@x"` is a *dotted key*, not a member named `foo.bar` —
        // which is why member keys must be quoted.
        let text = "version = 1\nid = \"bp-a3f9c2e1\"\n\n[members]\nfoo.bar = \"/repos/api\"\n";

        assert!(PlaneFile::parse(Path::new(PATH), text).is_err());
    }

    #[test]
    fn two_members_of_the_same_source_are_a_parse_error_naming_both_keys() {
        let text = concat!(
            "version = 1\nid = \"bp-a3f9c2e1\"\n\n[members]\n",
            "\"acme/codestyle\" = \"@codestyle\"\n",
            "\"other/codestyle\" = \"@codestyle\"\n",
        );

        let error = PlaneFile::parse(Path::new(PATH), text).unwrap_err();
        let message = error.envelope().message;

        assert!(message.contains("acme/codestyle"), "got {message}");
        assert!(message.contains("other/codestyle"), "got {message}");
    }

    #[test]
    fn a_version_bitplane_does_not_understand_is_refused_rather_than_guessed_at() {
        let text = "version = 2\nid = \"bp-a3f9c2e1\"\n\n[members]\n";

        let error = PlaneFile::parse(Path::new(PATH), text).unwrap_err();

        assert!(
            error.envelope().message.contains("version 2"),
            "got {}",
            error.envelope().message
        );
    }

    #[test]
    fn a_comment_a_user_wrote_survives_being_read() {
        let text = concat!(
            "# do not reap, long-running migration\n",
            "version = 1\nid = \"bp-a3f9c2e1\"\n\n[members]\n",
            "\"acme/codestyle\" = \"@codestyle\"\n",
        );

        let file = PlaneFile::parse(Path::new(PATH), text).unwrap();

        assert_eq!(file.members.len(), 1);
    }

    fn plane_file() -> PlaneFile {
        PlaneFile::new(
            PlaneId::parse("bp-a3f9c2e1").unwrap(),
            vec![
                Member {
                    path: WorktreePath::parse("acme/codestyle").unwrap(),
                    source: MemberRef::parse("@codestyle").unwrap(),
                },
                Member {
                    path: WorktreePath::parse("projects/bitplane").unwrap(),
                    source: MemberRef::Repo(PathBuf::from("/Users/alfonz/projects/bitplane")),
                },
            ],
        )
    }
}
