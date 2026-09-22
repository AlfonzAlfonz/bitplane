//! `bp list`, `bp show` and `bp status`, end to end through the binary.
//!
//! What the core tests cannot check: the rows a person reads, the same run's
//! machine rendering, the exit codes — including `3`, which is a **success**
//! that says "look at what this found" — and the two things that only exist
//! once there is a process: the working directory a plane is resolved from, and
//! the environment bitplane deliberately does not read back.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use bitplane_core::testing::{git, repository_with_one_commit, scratch_dir};

#[test]
fn a_listing_prints_a_block_per_plane_and_exits_ok() {
    let host = Host::new("cli-list");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    host.create(&[&alpha], "feat-a", "auth-work");
    host.create(&[&beta], "feat-b", "billing");

    let run = host.bp(&["list"]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert_eq!(stderr(&run), "");

    let printed = stdout(&run);
    assert!(printed.contains("auth-work"), "got:\n{printed}");
    assert!(printed.contains("billing"), "got:\n{printed}");
    assert!(
        printed.contains(&format!("  {}  feat-a\n", alpha.display())),
        "got:\n{printed}"
    );
}

#[test]
fn no_planes_at_all_is_ok_with_nothing_on_stdout() {
    let host = Host::new("cli-list-empty");

    let run = host.bp(&["list"]);

    assert_eq!(run.status.code(), Some(0));
    assert_eq!(stdout(&run), "");
    assert_eq!(stderr(&run), "");
}

#[test]
fn the_same_listing_under_json_is_one_line_of_the_response() {
    let host = Host::new("cli-list-json");
    let alpha = host.repository("alpha");
    host.create(&[&alpha], "feat-a", "auth-work");

    let run = host.bp(&["list", "--json"]);

    assert_eq!(run.status.code(), Some(0));
    assert_eq!(stderr(&run), "");
    assert_eq!(stdout(&run).lines().count(), 1);

    let json: serde_json::Value = serde_json::from_str(&stdout(&run)).unwrap();
    assert_eq!(json["action"], "plane_list");
    assert_eq!(json["planes"][0]["id"], "auth-work");
    assert_eq!(json["planes"][0]["members"][0]["head"]["branch"], "feat-a");
    assert_eq!(
        json["planes"][0]["health"]["findings"].as_array(),
        Some(&vec![])
    );
}

#[test]
fn a_finding_is_a_success_that_exits_three() {
    let host = Host::new("cli-list-drift");
    let alpha = host.repository("alpha");
    host.create(&[&alpha], "feat-a", "auth-work");
    fs::remove_dir_all(host.planes().join("auth-work").join("repos").join("alpha")).unwrap();

    let run = host.bp(&["list"]);

    assert_eq!(run.status.code(), Some(3), "stderr: {}", stderr(&run));
    assert_eq!(
        stderr(&run),
        "",
        "the result is on stdout; 3 says look at it"
    );
    assert!(
        stdout(&run).contains("worktree missing"),
        "got:\n{}",
        stdout(&run)
    );
}

#[test]
fn a_plane_file_that_will_not_parse_is_a_row_and_the_listing_carries_on() {
    let host = Host::new("cli-list-unreadable");
    let alpha = host.repository("alpha");
    host.create(&[&alpha], "feat-a", "zzz-healthy");
    let broken = host.planes().join("broken");
    fs::create_dir_all(&broken).unwrap();
    fs::write(
        broken.join("plane.toml"),
        "version = 1\nid = \"broken\"\nhost = \"local\"\n",
    )
    .unwrap();

    let run = host.bp(&["list"]);

    assert_eq!(run.status.code(), Some(3));
    let printed = stdout(&run);
    assert!(printed.contains("unknown key \"host\""), "got:\n{printed}");
    assert!(printed.contains("zzz-healthy"), "got:\n{printed}");
}

#[test]
fn show_describes_the_plane_the_working_directory_is_in() {
    let host = Host::new("cli-show-cwd");
    let alpha = host.repository("alpha");
    host.create(&[&alpha], "feat-a", "auth-work");

    let inside = host.planes().join("auth-work").join("repos").join("alpha");
    let run = host.bp_in(&inside, &["show"]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert_eq!(
        stdout(&run),
        format!(
            concat!(
                "auth-work\n",
                "  directory  {directory}\n",
                "{created}",
                "  members    1\n",
                "\n",
                "  {alpha}  feat-a  repos/alpha\n",
            ),
            directory = host.planes_as_resolved().join("auth-work").display(),
            created = created_line(&stdout(&run)),
            alpha = alpha.display(),
        )
    );
}

#[test]
fn the_plane_is_never_taken_from_the_environment_bitplane_writes() {
    let host = Host::new("cli-show-env");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    host.create(&[&alpha], "feat-a", "auth-work");
    host.create(&[&beta], "feat-b", "billing");

    let inside = host.planes().join("auth-work");
    let mut command = host.command(&["show"]);
    command
        .current_dir(&inside)
        // What `bp run` injects for a script to consume. Reading either back
        // would make `bp show` report a plane you are not standing in — and a
        // plane id is mutable, so it could name a different plane entirely.
        .env("BITPLANE_PLANE_ID", "billing")
        .env("BITPLANE_PLANE_DIR", host.planes().join("billing"));

    let run = command.output().expect("run bp");

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert!(
        stdout(&run).starts_with("auth-work\n"),
        "the walk from cwd wins: got\n{}",
        stdout(&run)
    );
}

#[test]
fn standing_outside_every_plane_is_a_usage_failure_naming_the_way_out() {
    let host = Host::new("cli-show-nowhere");

    let run = host.bp_in(&host.root, &["show"]);

    assert_eq!(run.status.code(), Some(2));
    assert_eq!(stdout(&run), "", "stdout carries results only");
    assert!(
        stderr(&run).starts_with("error[plane_not_found]: no plane contains "),
        "got:\n{}",
        stderr(&run)
    );
    assert!(
        stderr(&run).ends_with("remedy: cd into a plane, or name one with --plane.\n"),
        "got:\n{}",
        stderr(&run)
    );
}

#[test]
fn the_same_failure_under_json_carries_the_same_five_fields() {
    let host = Host::new("cli-show-nowhere-json");

    let run = host.bp_in(&host.root, &["show", "--json"]);

    assert_eq!(run.status.code(), Some(2));
    assert_eq!(stdout(&run), "");

    let envelope: serde_json::Value = serde_json::from_str(&stderr(&run)).unwrap();
    assert_eq!(envelope["error"], "plane_not_found");
    assert_eq!(envelope["code"], 2);
    assert_eq!(
        envelope["remedy"],
        "cd into a plane, or name one with --plane."
    );
}

#[test]
fn a_named_plane_is_shown_wherever_the_command_is_run_from() {
    let host = Host::new("cli-show-named");
    let alpha = host.repository("alpha");
    host.create(&[&alpha], "feat-a", "auth-work");

    let run = host.bp_in(&host.root, &["show", "-p", "auth-work"]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert!(stdout(&run).starts_with("auth-work\n"));
}

#[test]
fn show_names_the_finding_and_the_command_that_fixes_it() {
    let host = Host::new("cli-show-drift");
    let alpha = host.repository("alpha");
    host.create(&[&alpha], "feat-a", "auth-work");
    fs::remove_dir_all(host.planes().join("auth-work").join("repos").join("alpha")).unwrap();

    let run = host.bp(&["show", "-p", "auth-work"]);

    assert_eq!(run.status.code(), Some(3), "stderr: {}", stderr(&run));
    let printed = stdout(&run);
    assert!(printed.contains("  health     broken\n"), "got:\n{printed}");
    assert!(
        printed.ends_with(&format!(
            concat!(
                "findings\n",
                "  {alpha}  the worktree at repos/alpha is not there\n",
                "  {blank}  bp rm {alpha} drops it; bp add {alpha}:<branch> puts it back\n",
            ),
            alpha = alpha.display(),
            blank = " ".repeat(alpha.display().to_string().len()),
        )),
        "got:\n{printed}"
    );
}

#[test]
fn a_show_of_a_plane_file_that_will_not_parse_exits_one() {
    let host = Host::new("cli-show-unreadable");
    let broken = host.planes().join("broken");
    fs::create_dir_all(&broken).unwrap();
    fs::write(
        broken.join("plane.toml"),
        "version = 1\nid = \"broken\"\nhost = \"local\"\n",
    )
    .unwrap();

    let run = host.bp(&["show", "-p", "broken"]);

    // A listing tolerates a file it cannot read; a show has one file to read.
    assert_eq!(run.status.code(), Some(1), "stdout: {}", stdout(&run));
    assert!(
        stderr(&run).starts_with("error[parse_error]: "),
        "got:\n{}",
        stderr(&run)
    );
}

#[test]
fn status_renders_gits_own_answer_for_every_member() {
    let host = Host::new("cli-status");
    let alpha = host.repository("alpha");
    let beta = host.repository("beta");
    host.create(&[&alpha, &beta], "feat-a", "auth-work");

    let worktree = host.planes().join("auth-work").join("repos").join("alpha");
    fs::write(worktree.join("README.md"), "changed\n").unwrap();
    fs::write(worktree.join("notes.md"), "new\n").unwrap();

    let run = host.bp_in(&worktree, &["status"]);

    assert_eq!(run.status.code(), Some(0), "stderr: {}", stderr(&run));
    assert_eq!(
        stdout(&run),
        format!(
            concat!(
                "auth-work  {planes}/auth-work\n",
                "\n",
                "  {alpha}  feat-a  1 modified, 1 untracked, 1 ahead\n",
                "  {beta}   feat-a  clean, 1 ahead\n",
            ),
            planes = host.planes_as_resolved().display(),
            alpha = alpha.display(),
            beta = beta.display(),
        ),
        "uncommitted work is ordinary use and exits 0"
    );
}

#[test]
fn a_status_member_whose_worktree_is_gone_is_a_finding_at_exit_three() {
    let host = Host::new("cli-status-gone");
    let alpha = host.repository("alpha");
    host.create(&[&alpha], "feat-a", "auth-work");
    fs::remove_dir_all(host.planes().join("auth-work").join("repos").join("alpha")).unwrap();

    let run = host.bp(&["status", "-p", "auth-work"]);

    assert_eq!(run.status.code(), Some(3), "stderr: {}", stderr(&run));
    let printed = stdout(&run);
    assert!(
        printed.contains(&format!("  {}  -  worktree missing\n", alpha.display())),
        "got:\n{printed}"
    );
    assert!(printed.contains("findings\n"), "got:\n{printed}");
}

#[test]
fn asking_for_no_health_finds_nothing_and_exits_ok() {
    let host = Host::new("cli-health-none");
    let alpha = host.repository("alpha");
    host.create(&[&alpha], "feat-a", "auth-work");
    fs::remove_dir_all(host.planes().join("auth-work").join("repos").join("alpha")).unwrap();

    let run = host.bp(&["list", "--health", "none"]);

    assert_eq!(run.status.code(), Some(0), "nothing was looked for");
    assert!(
        !stdout(&run).contains("worktree missing"),
        "got:\n{}",
        stdout(&run)
    );
}

#[test]
fn the_full_tier_adds_what_only_git_knows() {
    let host = Host::new("cli-health-full");
    let alpha = host.repository("alpha");
    host.create(&[&alpha], "feat-a", "auth-work");
    fs::remove_dir_all(host.planes().join("auth-work").join("repos").join("alpha")).unwrap();

    let run = host.bp(&["show", "-p", "auth-work", "--health", "full"]);

    assert_eq!(run.status.code(), Some(3));
    assert!(
        stdout(&run).contains("worktree missing, prunable"),
        "got:\n{}",
        stdout(&run)
    );
}

#[test]
fn a_health_level_that_is_not_one_of_the_three_is_a_usage_failure() {
    let host = Host::new("cli-health-bogus");

    let run = host.bp(&["list", "--health", "thorough"]);

    assert_eq!(run.status.code(), Some(2));
    assert_eq!(stdout(&run), "");
    assert!(
        stderr(&run).starts_with("error[bad_usage]: "),
        "got: {}",
        stderr(&run)
    );
}

/// The `created` line, copied out of what was printed.
///
/// The plane directory's birth time is whatever the filesystem says, and it is
/// absent where the filesystem cannot say — so the fixture takes the line as
/// given and the surrounding assertion pins everything else exactly.
fn created_line(printed: &str) -> String {
    printed
        .lines()
        .find(|line| line.starts_with("  created "))
        .map(|line| format!("{line}\n"))
        .unwrap_or_default()
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

    fn planes(&self) -> PathBuf {
        self.root.join("planes")
    }

    /// The planes directory as a read reports it: one plane is one path, so the
    /// directory comes back canonical however it was reached.
    fn planes_as_resolved(&self) -> PathBuf {
        self.planes()
            .canonicalize()
            .expect("the planes directory exists by now")
    }

    fn create(&self, members: &[&Path], branch: &str, id: &str) {
        let mut arguments: Vec<String> = vec!["create".to_owned()];
        arguments.extend(members.iter().map(|member| member.display().to_string()));
        arguments.extend([
            "-b".to_owned(),
            branch.to_owned(),
            "--id".to_owned(),
            id.to_owned(),
        ]);

        let borrowed: Vec<&str> = arguments.iter().map(String::as_str).collect();
        let run = self.bp(&borrowed);

        assert_eq!(
            run.status.code(),
            Some(0),
            "the fixture plane was not created: {}",
            stderr(&run)
        );
    }

    fn bp(&self, arguments: &[&str]) -> Output {
        self.command(arguments).output().expect("run bp")
    }

    fn bp_in(&self, directory: &Path, arguments: &[&str]) -> Output {
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

/// Cuts the run off from the developer's own environment.
///
/// `--planes-dir` and `--projects-dir` are not enough on their own: the config
/// file is resolved and read whatever the flags say, so a real
/// `~/.config/bitplane/config.toml` with a key bitplane does not know would
/// fail every test here with an unrelated `parse_error`.
fn sealed_off(command: &mut Command, root: &Path) {
    command
        .env("BITPLANE_CONFIG", root.join("no-such-config.toml"))
        .env("XDG_CONFIG_HOME", root.join("xdg-config"))
        .env("XDG_DATA_HOME", root.join("xdg-data"))
        .env_remove("BITPLANE_PLANES_DIR")
        .env_remove("BITPLANE_PROJECTS_DIR");
}
