//! Rendering a string as TOML.
//!
//! bitplane writes its files by hand rather than by serialising, so that the
//! layout is the one a person would have typed — aligned keys, a blank line
//! before each table — and so a hand-edited file round-trips through
//! `toml_edit` with its comments intact.
//!
//! The one thing that must **not** be done by hand is quoting. Rust's `{:?}`
//! looks like it produces a TOML string and does not: it is `escape_debug`,
//! which spells a non-printable character `\u{301}` where TOML spells it
//! `́`. A combining accent is ordinary in a path on macOS, which stores
//! filenames decomposed — so the difference is not theoretical, and a file
//! written that way parses back as an error bitplane then cannot repair.
//!
//! `toml_edit` already knows the answer, and picking the literal or multi-line
//! form where those read better is a bonus rather than the point.

/// `value`, quoted as TOML.
pub fn quoted(value: &str) -> String {
    toml_edit::Value::from(value).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ordinary_string_is_quoted_the_obvious_way() {
        assert_eq!(quoted("codestyle"), "\"codestyle\"");
        assert_eq!(quoted("acme/codestyle"), "\"acme/codestyle\"");
    }

    #[test]
    fn what_is_quoted_parses_back_as_the_string_it_came_from() {
        let cases = [
            "codestyle",
            // A decomposed accent: what macOS stores for `é` in a filename,
            // and what `{:?}` renders as an escape TOML does not have.
            "e\u{301}clair",
            "with\na newline",
            "with\ta tab",
            "with \"quotes\"",
            "with\\a backslash",
            "git@gitlab.com:acme/codestyle.git",
            "/Users/alfonz/projects/odd name",
        ];

        for value in cases {
            let document: toml_edit::DocumentMut = format!("key = {}", quoted(value))
                .parse()
                .unwrap_or_else(|err| panic!("{value:?} rendered unparseable TOML: {err}"));

            assert_eq!(
                document["key"].as_str(),
                Some(value),
                "for {value:?}, rendered as {}",
                quoted(value)
            );
        }
    }

    #[test]
    fn rust_debug_formatting_is_not_toml_and_this_is_why() {
        // The regression this module exists for. Left as an assertion rather
        // than a comment so that a future "just use {:?}" has to argue with it.
        assert_eq!(format!("{:?}", "e\u{301}clair"), "\"e\\u{301}clair\"");
        assert!(
            format!("key = {:?}", "e\u{301}clair")
                .parse::<toml_edit::DocumentMut>()
                .is_err()
        );
    }
}
