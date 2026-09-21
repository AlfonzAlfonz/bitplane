//! `bp` — the human rendering of the bitplane contract.
//!
//! The CLI owns argument parsing and presentation and nothing else. Every
//! failure it reports is an [`ErrorEnvelope`] built in `bitplane-core`, written
//! to stderr as one line of JSON; stdout carries results only (ADR-0003).

use std::ffi::OsString;
use std::io::Write;

use bitplane_core::{ErrorEnvelope, LocalEngine, Termination, dispatch};
use clap::{Parser, Subcommand};

fn main() -> std::process::ExitCode {
    let termination = run(std::env::args_os());
    report(&termination, &mut std::io::stderr());
    termination.exit_code().into()
}

/// Manage planes: named sets of git worktrees that share one lifecycle.
#[derive(Debug, Parser)]
#[command(name = "bp", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

/// The verbs. Plane verbs are bare; project verbs are prefixed with `project`.
///
/// Empty today: this slice builds the frame every later one hangs a verb on.
#[derive(Debug, Subcommand)]
enum Command {}

/// Parses `args` and does what they ask.
fn run<I, T>(args: I) -> Termination
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    match Cli::try_parse_from(args) {
        Ok(cli) => execute(cli),
        Err(error) => render_parse_outcome(error),
    }
}

fn execute(cli: Cli) -> Termination {
    let Some(command) = cli.command else {
        return no_command().into();
    };

    // git is checked once, before any command touches anything (ADR-0001).
    let engine = LocalEngine::new();
    if let Err(error) = engine.ensure_git_supported() {
        return error.into();
    }

    let _response = dispatch(&engine, request_for(command));
    Termination::Ok
}

/// Clap reports `--help` and `--version` as errors; they are neither.
fn render_parse_outcome(error: clap::Error) -> Termination {
    use clap::error::ErrorKind;

    match error.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            // clap's own writer: help and version go to stdout, styled or not
            // according to the terminal.
            let _ = error.print();
            Termination::Ok
        }
        ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => no_command().into(),
        _ => ErrorEnvelope::usage("bad_usage", one_line(&error.render().to_string()))
            .with_remedy("Run `bp --help` to see what bp can do.")
            .into(),
    }
}

/// Writes the error envelope, where there is one.
fn report(termination: &Termination, stderr: &mut impl Write) {
    let Some(envelope) = termination.envelope() else {
        return;
    };

    let _ = writeln!(stderr, "{}", envelope.to_json());
}

/// `bp` on its own. Reported the same way whether clap or [`execute`] notices,
/// so the two can never disagree.
fn no_command() -> ErrorEnvelope {
    ErrorEnvelope::usage("no_command", "no command given")
        .with_remedy("Run `bp --help` to see what bp can do.")
}

/// Turns a parsed subcommand into the request that crosses the wire. Empty
/// while [`Command`] is: the `match` is what makes a forgotten wiring a
/// compile error.
fn request_for(command: Command) -> bitplane_core::Request {
    match command {}
}

/// Clap renders a usage error as several lines; the envelope's `message` is one
/// sentence.
fn one_line(rendered: &str) -> String {
    rendered
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("the command line could not be parsed")
        .trim_start_matches("error: ")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn no_arguments_is_a_usage_failure_naming_what_to_do() {
        let termination = run(["bp"]);
        let envelope = termination.envelope().expect("a usage failure");

        assert_eq!(envelope.error, "no_command");
        assert_eq!(envelope.code, bitplane_core::ExitCode::Usage);
        assert!(envelope.remedy.is_some());
    }

    #[test]
    fn an_unknown_flag_is_a_usage_failure_on_one_line() {
        let termination = run(["bp", "--nonsense"]);
        let envelope = termination.envelope().expect("a usage failure");

        assert_eq!(envelope.error, "bad_usage");
        assert_eq!(envelope.code, bitplane_core::ExitCode::Usage);
        assert!(
            !envelope.message.contains('\n'),
            "got: {}",
            envelope.message
        );
    }

    #[test]
    fn a_successful_termination_writes_nothing_to_stderr() {
        let mut stderr = Vec::new();
        report(&Termination::Ok, &mut stderr);

        assert!(stderr.is_empty());
    }

    #[test]
    fn a_failure_is_one_line_of_json_on_stderr() {
        let mut stderr = Vec::new();
        report(&run(["bp"]), &mut stderr);

        let written = String::from_utf8(stderr).unwrap();
        assert_eq!(written.lines().count(), 1);
        assert!(serde_json::from_str::<serde_json::Value>(&written).is_ok());
    }
}
