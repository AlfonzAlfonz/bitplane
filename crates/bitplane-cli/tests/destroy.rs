//! `bp destroy` and `bp rm`, end to end through the binary.
//!
//! What the core tests cannot check: the rows a person reads, the same run's
//! machine rendering, the exit codes, and that `--plane` and the current
//! directory name the same plane.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use bitplane_core::testing::{repository_with_an_origin, scratch_dir};

#[test]
fn a_destroyed_plane_prints_a_row_per_member_and_then_says_it_is_gone() {
    let host = Host::new("cli-destroy");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    host.plane(&[&alpha, &beta]);

    let run = host.bp(&["destroy", "-p", "auth-work"]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert_eq!(
        stdout(&run),
        format!(
            concat!(
                "auth-work  {planes}/auth-work\n",
                "\n",
                "  {alpha}  feat-login  removed  repos/alpha (branch kept)\n",
                "  {beta}   feat-login  removed  repos/beta (branch kept)\n",
                "\n",
                "destroyed auth-work\n",
            ),
            planes = host.planes_as_resolved().display(),
            alpha = alpha.display(),
            beta = beta.display(),
        )
    );
    assert_eq!(stderr(&run), "");
    assert!(!host.planes().join("auth-work").exists());
}

#[test]
fn the_same_run_under_json_puts_the_response_on_stdout_and_nothing_on_stderr() {
    let host = Host::new("cli-destroy-json");
    let alpha = host.repository("alpha");
    host.plane(&[&alpha]);

    let run = host.bp(&["destroy", "-p", "auth-work", "--json"]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert_eq!(stderr(&run), "");

    let response: serde_json::Value = serde_json::from_str(&stdout(&run)).unwrap();
    assert_eq!(response["action"], "plane_destroy");
    assert_eq!(response["members"][0]["outcome"], "ok");
    assert_eq!(response["members"][0]["detail"]["branch"], "feat-login");
    assert_eq!(response["incomplete"], false);
}

#[test]
fn a_refusal_exits_1_listing_every_reason_and_a_remedy_that_can_be_pasted() {
    let host = Host::new("cli-destroy-refused");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    host.plane(&[&alpha, &beta]);

    fs::write(host.worktree("repos/alpha").join("README.md"), "changed\n").unwrap();
    fs::write(host.worktree("repos/beta").join("scratch.txt"), "mine\n").unwrap();

    let run = host.bp(&["destroy", "-p", "auth-work"]);

    assert_eq!(run.status.code(), Some(1));
    assert_eq!(stdout(&run), "", "stdout carries results only");
    assert_eq!(
        stderr(&run),
        format!(
            concat!(
                "error[refused]: refusing to destroy auth-work: 2 of 2 members have work that \
                 would be lost\n",
                "\n",
                "  {alpha}  feat-login has uncommitted changes\n",
                "  {beta}   feat-login has untracked files\n",
                "\n",
                "remedy: Inspect the members listed. Re-run with --waive uncommitted \
                 --waive untracked to accept losing that work.\n",
            ),
            alpha = alpha.display(),
            beta = beta.display(),
        )
    );
    assert!(
        host.planes().join("auth-work").exists(),
        "nothing was touched"
    );
}

#[test]
fn a_waiver_is_echoed_in_the_row_it_was_granted_for() {
    let host = Host::new("cli-destroy-waived");
    let alpha = host.repository("alpha");
    host.plane(&[&alpha]);
    fs::write(host.worktree("repos/alpha").join("README.md"), "changed\n").unwrap();

    let run = host.bp(&["destroy", "-p", "auth-work", "--waive", "uncommitted"]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert!(
        stdout(&run).contains("repos/alpha (branch kept, waived: uncommitted)"),
        "got:\n{}",
        stdout(&run)
    );
}

#[test]
fn there_is_no_blanket_force_flag() {
    let host = Host::new("cli-destroy-no-force");
    let alpha = host.repository("alpha");
    host.plane(&[&alpha]);

    let run = host.bp(&["destroy", "-p", "auth-work", "--force"]);

    assert_eq!(run.status.code(), Some(2));
    assert!(host.planes().join("auth-work").exists());
}

#[test]
fn a_reason_that_is_not_a_reason_is_a_usage_failure_listing_the_ones_that_are() {
    let host = Host::new("cli-destroy-bad-waiver");
    let alpha = host.repository("alpha");
    host.plane(&[&alpha]);

    let run = host.bp(&["destroy", "-p", "auth-work", "--waive", "everything"]);

    assert_eq!(run.status.code(), Some(2));
    assert!(
        stderr(&run).contains("locked_worktree"),
        "the legal reasons have to be findable: {}",
        stderr(&run)
    );
}

#[test]
fn the_plane_defaults_to_the_one_the_current_directory_is_in() {
    let host = Host::new("cli-destroy-here");
    let alpha = host.repository("alpha");
    host.plane(&[&alpha]);

    let run = host.bp_in(host.worktree("repos/alpha"), &["destroy"]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert!(
        stdout(&run).starts_with("auth-work  "),
        "got:\n{}",
        stdout(&run)
    );
    assert!(!host.planes().join("auth-work").exists());
}

#[test]
fn standing_in_no_plane_at_all_exits_2_naming_the_way_out() {
    let host = Host::new("cli-destroy-nowhere");

    let run = host.bp_in(host.root.clone(), &["destroy"]);

    assert_eq!(run.status.code(), Some(2));
    assert!(
        stderr(&run).starts_with("error[plane_not_found]: no plane contains "),
        "got: {}",
        stderr(&run)
    );
    assert!(stderr(&run).contains("remedy: cd into a plane, or name one with --plane."));
}

#[test]
fn a_verb_that_names_its_plane_still_works_from_a_directory_that_is_gone() {
    // The shell `bp destroy` leaves you in when you ran it from inside the
    // plane. Only the verbs that fall back to the current directory need one,
    // so reading it before dispatch would break every other command in exactly
    // the shell this command just created.
    let host = Host::new("cli-destroy-gone-cwd");
    let alpha = host.repository("alpha");

    let run = host.bp_from_a_deleted_directory(&[
        "create",
        &alpha.display().to_string(),
        "-b",
        "feat-login",
        "--id",
        "auth-work",
    ]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert!(host.planes().join("auth-work").exists());
}

#[test]
fn rm_takes_the_member_named_and_leaves_the_plane_standing() {
    let host = Host::new("cli-rm");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    host.plane(&[&alpha, &beta]);

    let run = host.bp(&["rm", &alpha.display().to_string(), "-p", "auth-work"]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert_eq!(
        stdout(&run),
        format!(
            concat!(
                "auth-work  {planes}/auth-work\n",
                "\n",
                "  {alpha}  feat-login  removed  repos/alpha (branch kept)\n",
            ),
            planes = host.planes_as_resolved().display(),
            alpha = alpha.display(),
        )
    );
    assert!(!host.worktree("repos/alpha").exists());
    assert!(host.worktree("repos/beta").exists());
}

#[test]
fn rm_of_a_member_that_is_not_in_the_plane_exits_2() {
    let host = Host::new("cli-rm-stranger");
    let alpha = host.repository("alpha");
    let stranger = host.repository("stranger");
    host.plane(&[&alpha]);

    let run = host.bp(&["rm", &stranger.display().to_string(), "-p", "auth-work"]);

    assert_eq!(run.status.code(), Some(2));
    assert!(
        stderr(&run).starts_with("error[project_not_in_plane]: "),
        "got: {}",
        stderr(&run)
    );
    assert!(host.worktree("repos/alpha").exists());
}

#[test]
fn rm_with_no_member_named_is_a_usage_failure() {
    let host = Host::new("cli-rm-no-member");

    let run = host.bp(&["rm", "-p", "auth-work"]);

    assert_eq!(run.status.code(), Some(2));
}

// ---------------------------------------------------------------- fixtures --

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

    /// A repository whose `origin` already holds `main`, so `unpushed` is not
    /// the answer to every test here.
    fn repository(&self, name: &str) -> PathBuf {
        let path = self.root.join("repos").join(name);
        repository_with_an_origin(&path, &self.root.join("forge").join(name));
        path.canonicalize().expect("the repository was just made")
    }

    /// The plane every test in this file tears down, always `auth-work`.
    fn plane(&self, members: &[&Path]) {
        let mut arguments: Vec<String> = vec!["create".to_owned()];
        arguments.extend(members.iter().map(|member| member.display().to_string()));
        arguments.extend(["-b".to_owned(), "feat-login".to_owned()]);
        arguments.extend(["--id".to_owned(), "auth-work".to_owned()]);

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

    /// The planes directory as the engine reports it: a plane resolves to one
    /// path, so the directory comes back canonical however it was reached.
    fn planes_as_resolved(&self) -> PathBuf {
        self.planes()
            .canonicalize()
            .expect("the planes directory exists by now")
    }

    fn worktree(&self, relative: &str) -> PathBuf {
        self.planes().join("auth-work").join(relative)
    }

    fn bp(&self, arguments: &[&str]) -> Output {
        self.command(arguments).output().expect("run bp")
    }

    /// The same, run from somewhere — which is how `bp destroy` with no
    /// `--plane` finds the plane it is standing in.
    fn bp_in(&self, directory: PathBuf, arguments: &[&str]) -> Output {
        self.command(arguments)
            .current_dir(directory)
            .output()
            .expect("run bp")
    }

    /// `bp`, run from a directory that has been unlinked underneath it — which
    /// is what a shell sitting inside a plane you just destroyed looks like.
    ///
    /// Through `sh`, because the directory has to go *after* the `chdir` and
    /// before the exec, and a parent cannot arrange that for a child.
    fn bp_from_a_deleted_directory(&self, arguments: &[&str]) -> Output {
        let doomed = self.root.join("doomed");
        fs::create_dir_all(&doomed).unwrap();

        let mut script = format!(
            "cd {doomed} && rm -rf {doomed} && exec {bp}",
            doomed = doomed.display(),
            bp = env!("CARGO_BIN_EXE_bp"),
        );
        for argument in arguments {
            script.push_str(&format!(" '{argument}'"));
        }
        script.push_str(&format!(
            " --planes-dir '{}' --projects-dir '{}'",
            self.planes().display(),
            self.root.join("projects").display(),
        ));

        let mut command = Command::new("sh");
        command.arg("-c").arg(script);
        sealed_off(&mut command, &self.root);
        command.output().expect("run bp")
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
