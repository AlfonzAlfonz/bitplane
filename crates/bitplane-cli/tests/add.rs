//! `bp add`, end to end through the binary.
//!
//! What the core tests cannot check: the rows a person reads, the same run's
//! machine rendering, and the exit codes — including the `2` that
//! `--no-fetch` with the default branch intent has to produce before anything
//! is touched.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use bitplane_core::testing::{git, repository_with_one_commit, scratch_dir};

#[test]
fn an_added_member_prints_the_plane_header_and_its_own_row() {
    let host = Host::new("cli-add");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    host.plane(&[&alpha]);

    let run = host.bp(&[
        "add",
        &beta.display().to_string(),
        "-b",
        "feat-login",
        "-p",
        "auth-work",
    ]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert_eq!(
        stdout(&run),
        format!(
            concat!(
                "auth-work  {planes}/auth-work\n",
                "\n",
                "  {beta}  feat-login  created (new branch)  repos/beta\n",
            ),
            planes = host.planes_as_resolved().display(),
            beta = beta.display(),
        ),
        "members already in the plane are not reprinted; add reports what it did"
    );
    assert_eq!(stderr(&run), "");
}

#[test]
fn the_same_run_under_json_puts_the_response_on_stdout_and_nothing_on_stderr() {
    let host = Host::new("cli-add-json");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    host.plane(&[&alpha]);

    let run = host.bp(&[
        "add",
        &beta.display().to_string(),
        "-b",
        "feat-login",
        "-p",
        "auth-work",
        "--json",
    ]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert_eq!(stderr(&run), "");

    let response: serde_json::Value = serde_json::from_str(&stdout(&run)).unwrap();
    assert_eq!(response["action"], "plane_add");
    assert_eq!(response["id"], "auth-work");
    assert_eq!(response["members"][0]["outcome"], "ok");
    assert_eq!(response["members"][0]["detail"]["branch"], "feat-login");
}

#[test]
fn a_member_already_in_the_plane_exits_2_naming_the_two_commands_that_fix_it() {
    let host = Host::new("cli-add-duplicate");
    let alpha = host.repository("alpha");
    host.plane(&[&alpha]);

    let run = host.bp(&[
        "add",
        &alpha.display().to_string(),
        "-b",
        "feat-other",
        "-p",
        "auth-work",
    ]);

    assert_eq!(run.status.code(), Some(2));
    assert_eq!(stdout(&run), "", "stdout carries results only");
    assert_eq!(
        stderr(&run),
        format!(
            concat!(
                "error[duplicate_member]: {alpha} is already a member of auth-work\n",
                "\n",
                "remedy: Run bp rm {alpha}, then bp add {alpha}:<branch>.\n",
            ),
            alpha = alpha.display(),
        )
    );
}

#[test]
fn no_fetch_with_the_default_intent_exits_2_naming_both_ways_out() {
    let host = Host::new("cli-add-no-fetch");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    host.plane(&[&alpha]);

    let run = host.bp(&[
        "add",
        &beta.display().to_string(),
        "-b",
        "feat-x",
        "-p",
        "auth-work",
        "--no-fetch",
    ]);

    assert_eq!(run.status.code(), Some(2));
    assert_eq!(
        stderr(&run),
        concat!(
            "error[branch_intent_requires_fetch]: --no-fetch cannot be combined with the \
             default branch intent\n",
            "\n",
            "remedy: Drop --no-fetch, or pass --new-branch to create feat-x deliberately.\n",
        )
    );
    assert!(!host.worktree("repos/beta").exists(), "nothing was touched");
}

#[test]
fn add_with_no_branch_at_all_is_a_usage_failure() {
    let host = Host::new("cli-add-no-branch");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    host.plane(&[&alpha]);

    let run = host.bp(&["add", &beta.display().to_string(), "-p", "auth-work"]);

    assert_eq!(run.status.code(), Some(2));
    assert!(
        stderr(&run).contains("error[branch_unspecified]"),
        "got {}",
        stderr(&run)
    );
}

#[test]
fn the_plane_defaults_to_the_one_the_current_directory_is_in() {
    let host = Host::new("cli-add-cwd");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    host.plane(&[&alpha]);

    let run = host.bp_in(
        host.worktree("repos/alpha"),
        &["add", &beta.display().to_string(), "-b", "feat-login"],
    );

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert!(host.worktree("repos/beta").is_dir());
}

#[test]
fn add_with_no_member_named_is_a_usage_failure() {
    let host = Host::new("cli-add-no-member");
    let alpha = host.repository("alpha");
    host.plane(&[&alpha]);

    assert_eq!(
        host.bp(&["add", "-p", "auth-work", "-b", "feat"])
            .status
            .code(),
        Some(2)
    );
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

fn stdout(run: &Output) -> String {
    String::from_utf8(run.stdout.clone()).expect("stdout is utf-8")
}

fn stderr(run: &Output) -> String {
    String::from_utf8(run.stderr.clone()).expect("stderr is utf-8")
}

/// A host of its own, so no test can see another's planes.
struct Host {
    root: PathBuf,
}

impl Host {
    fn new(label: &str) -> Host {
        Host {
            root: scratch_dir(label),
        }
    }

    fn repository(&self, name: &str) -> PathBuf {
        let path = self.root.join("repos").join(name);
        repository_with_one_commit(&path);
        // A branch for a plane to land on that the checkout does not occupy.
        git(&path, &["branch", "spare"]);
        path.canonicalize().expect("the repository was just made")
    }

    fn plane(&self, members: &[&Path]) {
        let mut arguments = vec!["create".to_owned()];
        arguments.extend(members.iter().map(|member| member.display().to_string()));
        arguments.extend(["-b", "feat-login", "--id", "auth-work"].map(str::to_owned));

        let run = self.bp(&arguments.iter().map(String::as_str).collect::<Vec<&str>>());
        assert!(
            run.status.success(),
            "the fixture plane was not built: {}",
            stderr(&run)
        );
    }

    fn planes(&self) -> PathBuf {
        self.root.join("planes")
    }

    /// The planes directory as the engine resolves it — `add` walks to the
    /// plane through `plane.toml`, and on macOS `/tmp` is a symlink.
    fn planes_as_resolved(&self) -> PathBuf {
        self.planes()
            .canonicalize()
            .expect("the planes directory exists once a plane has been built")
    }

    fn worktree(&self, relative: &str) -> PathBuf {
        self.planes().join("auth-work").join(relative)
    }

    fn bp(&self, arguments: &[&str]) -> Output {
        self.command(arguments).output().expect("run bp")
    }

    fn bp_in(&self, directory: PathBuf, arguments: &[&str]) -> Output {
        self.command(arguments)
            .current_dir(directory)
            .output()
            .expect("run bp")
    }

    fn command(&self, arguments: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_bp"));
        command
            .args(arguments)
            .arg("--planes-dir")
            .arg(self.planes())
            .arg("--projects-dir")
            .arg(self.root.join("projects"));
        sealed_off(&mut command, &self.root);
        command
    }
}
