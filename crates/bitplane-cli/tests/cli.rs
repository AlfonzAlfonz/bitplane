//! What `bp` does before it does anything: report itself, refuse an empty
//! invocation, and keep stdout clean for results.
//!
//! Both renderings are asserted from the same invocation, because every line of
//! the human one maps to exactly one field of the envelope — that is the
//! property that keeps them from drifting apart.

use std::process::{Command, Output};

#[test]
fn version_is_reported_on_stdout_and_exits_ok() {
    let run = bp(&["--version"]);

    assert_eq!(run.status.code(), Some(0));
    assert!(
        stdout(&run).starts_with("bp "),
        "expected a `bp <version>` line, got: {}",
        stdout(&run)
    );
    assert_eq!(stderr(&run), "");
}

#[test]
fn help_is_printed_on_stdout_and_exits_ok() {
    let run = bp(&["--help"]);

    assert_eq!(run.status.code(), Some(0));
    assert!(stdout(&run).contains("bp"), "help should name the binary");
    assert_eq!(stderr(&run), "");
}

#[test]
fn no_arguments_is_a_usage_error_rendered_for_a_human() {
    let run = bp(&[]);

    assert_eq!(run.status.code(), Some(2));
    assert_eq!(stdout(&run), "", "stdout carries results only");
    assert_eq!(
        stderr(&run),
        concat!(
            "error[no_command]: no command given\n",
            "\n",
            "remedy: Run `bp --help` to see what bp can do.\n",
        )
    );
}

#[test]
fn the_same_failure_under_json_is_one_line_of_the_envelope() {
    let run = bp(&["--json"]);

    assert_eq!(run.status.code(), Some(2));
    assert_eq!(stdout(&run), "", "stdout carries results only");

    let envelope = envelope(&run);
    assert_eq!(envelope["code"], 2);
    assert_eq!(envelope["error"], "no_command");
    assert!(
        envelope["remedy"].is_string(),
        "a usage error must say what to do instead"
    );
}

#[test]
fn an_unknown_flag_is_a_usage_error() {
    let run = bp(&["--nonsense", "--json"]);

    assert_eq!(run.status.code(), Some(2));
    assert_eq!(stdout(&run), "", "stdout carries results only");
    assert_eq!(envelope(&run)["code"], 2);
    assert_eq!(envelope(&run)["error"], "bad_usage");
}

#[test]
fn a_failure_envelope_carries_every_field_of_the_contract() {
    let run = bp(&["--json"]);
    let envelope = envelope(&run);

    for field in ["error", "code", "message", "problems", "remedy"] {
        assert!(
            envelope.get(field).is_some(),
            "envelope is missing `{field}`: {envelope}"
        );
    }
    assert!(envelope["problems"].is_array());
}

#[test]
fn a_json_after_the_separator_is_a_member_not_the_mode() {
    // `--` ends the flags, so what follows is a value. Choosing the rendering
    // by scanning raw argv would read this one as the mode and answer in JSON.
    let run = bp(&["create", "--", "--json"]);

    assert_eq!(run.status.code(), Some(2));
    assert!(
        stderr(&run).starts_with("error["),
        "the failure must stay human: {}",
        stderr(&run)
    );
}

#[test]
fn create_names_every_flag_the_reference_documents() {
    let help = stdout(&bp(&["create", "--help"]));

    for flag in [
        "--branch",
        "--id",
        "--new-branch",
        "--existing-branch",
        "--planes-dir",
        "--projects-dir",
        "--config",
        "--json",
    ] {
        assert!(
            help.contains(flag),
            "`bp create --help` omits {flag}:\n{help}"
        );
    }
}

/// `bp`, cut off from the developer's own config: resolution reads it whatever
/// the flags say, so an unrelated key in a real one would fail these tests.
fn bp(args: &[&str]) -> Output {
    let nowhere = std::env::temp_dir().join("bitplane-tests-no-such-config");

    Command::new(env!("CARGO_BIN_EXE_bp"))
        .args(args)
        .env("BITPLANE_CONFIG", nowhere.join("config.toml"))
        .env("XDG_CONFIG_HOME", nowhere.join("xdg-config"))
        .env("XDG_DATA_HOME", nowhere.join("xdg-data"))
        .env_remove("BITPLANE_PLANES_DIR")
        .env_remove("BITPLANE_PROJECTS_DIR")
        .output()
        .expect("run bp")
}

/// The JSON error envelope `bp` writes to stderr on failure under `--json`.
fn envelope(run: &Output) -> serde_json::Value {
    serde_json::from_str(&stderr(run))
        .unwrap_or_else(|err| panic!("stderr is not a JSON envelope ({err}): {}", stderr(run)))
}

fn stdout(run: &Output) -> String {
    String::from_utf8(run.stdout.clone()).expect("stdout is utf-8")
}

fn stderr(run: &Output) -> String {
    String::from_utf8(run.stderr.clone()).expect("stderr is utf-8")
}
