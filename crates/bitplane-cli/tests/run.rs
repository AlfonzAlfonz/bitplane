//! `bp run`, end to end through the binary.
//!
//! What the core tests cannot check: which stream a script's output lands on.
//! **Nothing script-related is ever written to stdout** — stdout is the machine
//! contract, and a `pnpm` banner inside the JSON is a broken contract — so the
//! interesting assertions here are all about the split.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use bitplane_core::testing::{repository_with_one_commit, scratch_dir};

#[test]
fn a_script_run_prints_a_row_per_script_and_its_output_goes_to_stderr() {
    let host = Host::new("cli-run");
    host.project("api");
    host.declares(
        "api",
        concat!(
            "[scripts.link-alfonz]\nshell = \"echo linking\"\n\n",
            "[scripts.install]\nshell = \"echo installing >&2\"\n",
        ),
    );
    host.plane();

    let run = host.bp(&["run", "@api", "link-alfonz", "install"]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert_eq!(
        strip_durations(&stdout(&run)),
        concat!(
            "@api  link-alfonz  ok  <duration>\n",
            "@api  install      ok  <duration>\n",
        ),
        "request order, and nothing about the plane"
    );
    assert!(
        stderr(&run).contains("linking") && stderr(&run).contains("installing"),
        "both streams are merged onto stderr: {}",
        stderr(&run)
    );
}

#[test]
fn the_same_run_under_json_still_keeps_the_script_output_off_stdout() {
    let host = Host::new("cli-run-json");
    host.project("api");
    host.declares("api", "[scripts.install]\nshell = \"echo installing\"\n");
    host.plane();

    let run = host.bp(&["run", "@api", "install", "--json"]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    let response: serde_json::Value = serde_json::from_str(&stdout(&run))
        .unwrap_or_else(|err| panic!("stdout is not one line of JSON ({err}): {}", stdout(&run)));
    assert_eq!(response["action"], "plane_scripts");
    assert_eq!(response["scripts"][0]["name"], "install");
    assert_eq!(response["scripts"][0]["project"], "api");
    assert_eq!(response["scripts"][0]["result"], "ok");
    assert!(response["scripts"][0]["log"].is_string());
    assert!(response["scripts"][0]["finished_at"].is_string());
    assert!(stderr(&run).contains("installing"));
}

#[test]
fn a_script_that_fails_exits_1_and_names_its_log() {
    let host = Host::new("cli-run-failure");
    host.project("api");
    host.declares(
        "api",
        concat!(
            "[scripts.install]\nshell = \"exit 1\"\n\n",
            "[scripts.after]\nshell = \"echo never\"\n",
        ),
    );
    host.plane();

    let run = host.bp(&["run", "@api", "install", "after"]);

    assert_eq!(run.status.code(), Some(1));
    assert_eq!(stdout(&run), "", "stdout carries results only");
    assert!(
        stderr(&run).starts_with("error[script_failed]: install exited 1 in @api\n"),
        "got: {}",
        stderr(&run)
    );
    assert!(
        stderr(&run).contains(".bitplane/logs/"),
        "the problem names the log: {}",
        stderr(&run)
    );
    assert!(
        stderr(&run)
            .ends_with("remedy: See the log, fix the cause, then run bp run @api install.\n"),
        "got: {}",
        stderr(&run)
    );
    assert!(!stderr(&run).contains("never"), "later scripts are not run");
}

#[test]
fn an_unknown_script_is_a_usage_failure_naming_where_to_look() {
    let host = Host::new("cli-run-unknown");
    host.project("api");
    host.declares("api", "[scripts.install]\nshell = \"true\"\n");
    host.plane();

    let run = host.bp(&["run", "@api", "intall"]);

    assert_eq!(run.status.code(), Some(2));
    assert!(
        stderr(&run).starts_with("error[script_not_found]: @api has no script called intall\n"),
        "got: {}",
        stderr(&run)
    );
}

#[test]
fn the_project_is_mandatory_and_so_is_a_script_name() {
    let host = Host::new("cli-run-usage");

    for arguments in [vec!["run"], vec!["run", "@api"]] {
        let run = host.bp(&arguments);

        assert_eq!(run.status.code(), Some(2), "for {arguments:?}");
    }
}

#[test]
fn the_sigil_is_optional_in_the_one_slot_that_takes_no_path() {
    let host = Host::new("cli-run-bare-name");
    host.project("api");
    host.declares("api", "[scripts.install]\nshell = \"true\"\n");
    host.plane();

    let run = host.bp(&["run", "api", "install"]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert!(
        stdout(&run).starts_with("@api  install"),
        "got {}",
        stdout(&run)
    );
}

// ---------------------------------------------------------------- fixtures --

/// A host of its own, holding one project and one plane called `auth-work`.
struct Host {
    root: PathBuf,
}

impl Host {
    fn new(label: &str) -> Host {
        Host {
            root: scratch_dir(label),
        }
    }

    /// An owned project, registered from a local repository standing in for a
    /// forge — a path is a git URL, so this is the code path a real one takes.
    fn project(&self, name: &str) {
        let forge = self.root.join("forges").join(name);
        repository_with_one_commit(&forge);

        let run = self.bp(&["project", "add", &forge.display().to_string()]);
        assert!(
            run.status.success(),
            "the fixture project was not registered: {}",
            stderr(&run)
        );
    }

    /// Appends a `[scripts]` table to the project's file, the way a user does:
    /// bitplane never writes one on their behalf.
    fn declares(&self, project: &str, scripts: &str) {
        let file = self.projects().join(project).join("project.toml");
        let text = fs::read_to_string(&file).expect("the project was registered first");

        fs::write(file, format!("{text}\n{scripts}")).expect("write the project file");
    }

    /// The plane every test in this file runs inside, always `auth-work`.
    fn plane(&self) {
        let run = self.bp(&["create", "@api", "-b", "feat-login", "--id", "auth-work"]);
        assert!(
            run.status.success(),
            "the fixture plane was not built: {}",
            stderr(&run)
        );
    }

    fn planes(&self) -> PathBuf {
        self.root.join("planes")
    }

    fn projects(&self) -> PathBuf {
        self.root.join("projects")
    }

    /// Run from inside the plane, which is how `bp run` finds it without
    /// `--plane`.
    fn bp(&self, arguments: &[&str]) -> Output {
        let inside = self.planes().join("auth-work");
        let mut command = Command::new(env!("CARGO_BIN_EXE_bp"));
        command
            .args(arguments)
            .arg("--planes-dir")
            .arg(self.planes())
            .arg("--projects-dir")
            .arg(self.projects());
        if inside.is_dir() {
            command.current_dir(inside);
        }
        sealed_off(&mut command, &self.root);

        command.output().expect("run bp")
    }
}

/// Cuts the run off from the developer's own environment.
fn sealed_off(command: &mut Command, root: &Path) {
    command
        .env("BITPLANE_CONFIG", root.join("no-such-config.toml"))
        .env("XDG_CONFIG_HOME", root.join("xdg-config"))
        .env("XDG_DATA_HOME", root.join("xdg-data"))
        .env_remove("BITPLANE_PLANES_DIR")
        .env_remove("BITPLANE_PROJECTS_DIR");
}

/// How long a script took is real elapsed time, so the rows are compared with
/// the last column taken out — the same thing the reference suite normalises.
/// Only the last column: the widths of the others are the assertion.
fn strip_durations(text: &str) -> String {
    text.lines()
        .map(|line| match line.rsplit_once("  ") {
            Some((head, took)) if took.strip_suffix('s').is_some_and(is_a_number) => {
                format!("{head}  <duration>\n")
            }
            _ => format!("{line}\n"),
        })
        .collect()
}

fn is_a_number(text: &str) -> bool {
    text.parse::<f64>().is_ok()
}

fn stdout(run: &Output) -> String {
    String::from_utf8(run.stdout.clone()).expect("stdout is utf-8")
}

fn stderr(run: &Output) -> String {
    String::from_utf8(run.stderr.clone()).expect("stderr is utf-8")
}
