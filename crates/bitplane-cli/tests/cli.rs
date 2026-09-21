//! What `bp` does before it does anything: report itself, refuse an empty
//! invocation, and keep stdout clean for results.

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
fn no_arguments_is_a_usage_error() {
    let run = bp(&[]);

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
    let run = bp(&["--nonsense"]);

    assert_eq!(run.status.code(), Some(2));
    assert_eq!(stdout(&run), "", "stdout carries results only");
    assert_eq!(envelope(&run)["code"], 2);
}

#[test]
fn a_failure_envelope_carries_every_field_of_the_contract() {
    let run = bp(&[]);
    let envelope = envelope(&run);

    for field in ["error", "code", "message", "problems", "remedy"] {
        assert!(
            envelope.get(field).is_some(),
            "envelope is missing `{field}`: {envelope}"
        );
    }
    assert!(envelope["problems"].is_array());
}

fn bp(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bp"))
        .args(args)
        .output()
        .expect("run bp")
}

/// The JSON error envelope `bp` writes to stderr on failure.
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
