//! What a project declares, and running it (ADR-0007).
//!
//! A **script** is a named user-declared command. It may be bound to zero or
//! more lifecycle points and is always runnable on demand, which is why the
//! word "hook" left the vocabulary: a hook fires at a point, and half of these
//! never do.
//!
//! ```toml
//! [scripts.link-alfonz]
//! argv = ["ln", "-s", "../../.alfonz", ".alfonz"]
//! post_worktree_create = true
//!
//! [scripts.install]
//! shell = "pnpm i && pnpm build"
//! post_worktree_create = true
//! ```
//!
//! **Execution order is declaration order**, which is semantic rather than
//! cosmetic: scripts run sequentially, so `link-alfonz` landing before
//! `install` is what lets the build read `.alfonz`. bitplane never rewrites
//! `[scripts]`, so that order cannot change behind the user's back.
//!
//! **Pre-scripts block and short-circuit; post-scripts do neither** — the rule
//! that makes teardown trustworthy. It is enforced by the caller through
//! [`Stop`], because only the caller knows which side of a destructive act it
//! is standing on.
//!
//! Everything here is **sequential**: across members in plane file order, and
//! within a member in declaration order. ADR-0003's parallel git fan-out does
//! not extend to arbitrary user commands — two `pnpm i` racing on one shared
//! store is the obvious first bug — and sequential is also what makes the live
//! output legible, since exactly one script writes at a time.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::directories::Directories;
use crate::error::EngineError;
use crate::head::Head;
use crate::interrupt::Interrupt;
use crate::member::{MemberRef, PROJECT_SIGIL, ProjectName};
use crate::plane_dir::BITPLANE_DIR;
use crate::plane_file::Member;
use crate::plane_id::is_well_formed_id;
use crate::project_dir::{self, BIN_DIR_NAME, ProjectDirectory};
use crate::time::Rfc3339;
use crate::wire::{ScriptOutcome, ScriptResult};

/// Where a plane keeps its script logs, under its reserved directory.
pub const LOGS_DIR: &str = "logs";

/// The shell a `shell = "…"` script is handed to. POSIX only, per ADR-0001.
const SHELL: &str = "/bin/sh";

/// A script's name: the same character set as a plane id, so a user-chosen name
/// is never trusted into a path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct ScriptName(String);

/// One of the two moments bitplane fires scripts at.
///
/// Named for the **worktree** rather than for the plane, because scripts are
/// per-project: `post_worktree_create` fires per member on both `create` and
/// `add`, and `pre_worktree_remove` on both `destroy` and `rm`. Both are the
/// identical situation seen twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ScriptPoint {
    /// After a worktree has been created and the plane is complete.
    PostWorktreeCreate,
    /// Before a worktree is removed, and before anything has been.
    PreWorktreeRemove,
}

/// What a script actually runs.
///
/// `argv` by default, so bitplane never has to invent quoting rules; the shell
/// form exists because the real examples want a shell and pretending otherwise
/// just makes everyone write `["sh", "-c", …]`. A bare string shorthand is
/// rejected deliberately: a list of strings reads like argv and would mean
/// shell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptCommand {
    Argv(Vec<String>),
    Shell(String),
}

/// One `[scripts.<name>]` table, as the file holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Script {
    pub name: ScriptName,
    #[serde(flatten)]
    pub command: ScriptCommand,
    /// The points this script is bound to. Empty is legal and useful: a script
    /// bound to nothing is still runnable with `bp run`.
    pub points: Vec<ScriptPoint>,
}

/// The plane a script runs inside.
#[derive(Debug, Clone, Copy)]
pub struct InPlane<'a> {
    pub id: &'a str,
    pub directory: &'a Path,
}

/// Everything running a script needs that is not in the script.
pub struct ScriptContext<'a> {
    pub directories: &'a Directories,
    pub interrupt: Interrupt,
    /// Where merged script output goes as it arrives.
    ///
    /// A sink rather than a `write!` to stderr, for the reason
    /// [`crate::LocalEngine`]'s lock announcement is one: the engine does not
    /// own a terminal, and a surface that wants the bytes says so.
    pub output: &'a (dyn Fn(&[u8]) + Sync),
}

/// Whether a pass stops at the first script that fails.
///
/// The whole of "pre blocks and short-circuits, post does neither", spelled
/// where the caller can see which one it asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// A non-zero exit ends the pass. Every pre-script pass, and `bp run`.
    AtTheFirstFailure,
    /// Every script runs, and the failures are reported afterwards. Every
    /// post-script pass, where by construction there is nothing left to stop.
    Never,
}

/// Where one project's scripts run, and what it declares.
///
/// Absent for a member with no `project.toml` — an **ad-hoc member**, or a
/// `@name` nothing registered — which is what makes "nothing runs, nothing is
/// reported" fall out of the ordinary path instead of being a special case.
pub struct Site {
    pub project: ProjectName,
    pub project_directory: PathBuf,
    pub source_repo: PathBuf,
    pub worktree: PathBuf,
    pub plane_directory: PathBuf,
    pub plane_id: String,
    /// In declaration order, which is execution order.
    pub scripts: Vec<Script>,
}

/// Runs every script bound to `point`, member by member in the order given.
///
/// The outcomes are the result; whether any of them failing is an error is the
/// caller's to decide, because a blocked teardown and a failed setup are
/// different news. A member whose worktree is not there is skipped entirely:
/// the point is named for the worktree, and there is no worktree.
pub fn at_point(
    point: ScriptPoint,
    members: &[Member],
    plane: InPlane<'_>,
    stop: Stop,
    context: &ScriptContext<'_>,
) -> Result<Vec<ScriptOutcome>, EngineError> {
    let mut outcomes = Vec::new();

    for member in members {
        let Some(site) = site_of(
            &member.source,
            member.path.under(plane.directory),
            plane,
            context,
        )?
        else {
            continue;
        };
        if !site.worktree.is_dir() {
            continue;
        }

        let bound: Vec<Script> = site
            .scripts
            .iter()
            .filter(|script| script.points.contains(&point))
            .cloned()
            .collect();

        if run_each(&site, &bound, stop, &mut outcomes, context)? {
            break;
        }
    }

    Ok(outcomes)
}

/// Runs the scripts named, for the projects named, in request order.
///
/// Request order rather than declaration order: unlike a lifecycle point, the
/// caller named these, so that is the order they meant. A project's scripts
/// stay contiguous, because sequencing within one worktree is the thing
/// declaration order exists to protect.
pub fn named(
    projects: &[ProjectName],
    names: &[ScriptName],
    members: &[Member],
    plane: InPlane<'_>,
    context: &ScriptContext<'_>,
) -> Result<Vec<ScriptOutcome>, EngineError> {
    let sites = resolve_named(projects, names, members, plane, context)?;
    let mut outcomes = Vec::new();

    for (site, wanted) in &sites {
        if run_each(
            site,
            wanted,
            Stop::AtTheFirstFailure,
            &mut outcomes,
            context,
        )? {
            break;
        }
    }

    Ok(outcomes)
}

/// Every project and script `bp run` was asked for, checked before any of them
/// runs.
///
/// Both checks happen up front and are distinct typed errors, so naming one
/// project that is not a member never leaves half a run behind.
fn resolve_named(
    projects: &[ProjectName],
    names: &[ScriptName],
    members: &[Member],
    plane: InPlane<'_>,
    context: &ScriptContext<'_>,
) -> Result<Vec<(Site, Vec<Script>)>, EngineError> {
    let mut resolved = Vec::new();

    for project in projects {
        let reference = MemberRef::Project(project.clone());
        let member = members
            .iter()
            .find(|member| member.source == reference)
            .ok_or_else(|| EngineError::ProjectNotInPlane {
                member: reference.to_string(),
                plane: plane.id.to_owned(),
            })?;

        // `Site` is absent where there is no `project.toml`. A member the plane
        // names as a project and the registry does not hold is not an ad-hoc
        // member — `bp run` was pointed at a project, so say so.
        let site = site_of(
            &member.source,
            member.path.under(plane.directory),
            plane,
            context,
        )?
        .ok_or_else(|| EngineError::ProjectNotFound {
            name: project.to_string(),
        })?;

        let mut wanted = Vec::new();
        for name in names {
            let script = site
                .scripts
                .iter()
                .find(|script| &script.name == name)
                .ok_or_else(|| EngineError::ScriptNotFound {
                    project: project.to_string(),
                    name: name.to_string(),
                })?;
            wanted.push(script.clone());
        }

        resolved.push((site, wanted));
    }

    Ok(resolved)
}

/// One site's scripts, in order. Answers whether the pass should stop.
fn run_each(
    site: &Site,
    scripts: &[Script],
    stop: Stop,
    into: &mut Vec<ScriptOutcome>,
    context: &ScriptContext<'_>,
) -> Result<bool, EngineError> {
    for script in scripts {
        // A script killed mid-run leaves no record at all: `finished_at` is not
        // optional, so "started but never finished" is unrepresentable and
        // Ctrl-C is the escape (ADR-0004).
        if context.interrupt.is_raised() {
            return Ok(true);
        }

        let outcome = run_one(site, script, context)?;
        let failed = !outcome.succeeded();
        into.push(outcome);

        if failed && stop == Stop::AtTheFirstFailure {
            return Ok(true);
        }
    }

    Ok(false)
}

/// One script: spawned in its worktree, its output merged, streamed and tee'd.
fn run_one(
    site: &Site,
    script: &Script,
    context: &ScriptContext<'_>,
) -> Result<ScriptOutcome, EngineError> {
    let log = log_path(site, &script.name)?;
    // Appended, never truncated. The stamp resolves to a second, so two runs of
    // one script inside the same second name the same file — and the remedy for
    // a failed script points the user at that file, so the retry must not be
    // what destroys it.
    let mut file = File::options()
        .create(true)
        .append(true)
        .open(&log)
        .map_err(|err| EngineError::io(&log, err))?;
    let started = Instant::now();

    let result = match spawn(site, script, &mut file, context) {
        Ok(result) => result,
        // Nothing ran, so there is nothing for the log to hold but the reason.
        Err(message) => {
            let _ = writeln!(file, "{message}");
            (context.output)(format!("{message}\n").as_bytes());
            ScriptResult::Failed {
                code: None,
                detail: format!("could not be run: {message}"),
            }
        }
    };

    Ok(ScriptOutcome {
        name: script.name.to_string(),
        project: Some(site.project.clone()),
        log,
        finished_at: Rfc3339::now().to_string(),
        duration_ms: started.elapsed().as_millis() as u64,
        result,
    })
}

/// Runs the child to completion, or says why it never started.
///
/// **stdout and stderr are merged into one pipe**, so the log and the live
/// stream hold the same bytes in the same order the script wrote them. Merged
/// onto bitplane's stderr and never its stdout: stdout is the machine contract,
/// and a `pnpm` banner inside the JSON is a broken contract.
fn spawn(
    site: &Site,
    script: &Script,
    log: &mut File,
    context: &ScriptContext<'_>,
) -> Result<ScriptResult, String> {
    let (program, arguments) = script.command.invocation();

    let (reader, writer) = io::pipe().map_err(|err| err.to_string())?;
    let mut command = Command::new(program);
    command
        .args(arguments)
        .current_dir(&site.worktree)
        .stdin(Stdio::null())
        .stdout(Stdio::from(
            writer.try_clone().map_err(|err| err.to_string())?,
        ))
        .stderr(Stdio::from(writer));
    environment(&mut command, site, &script.name)?;

    let mut child = command.spawn().map_err(|err| err.to_string())?;
    // The parent's ends of the pipe live in `command` until it is dropped, and
    // a read of a pipe nobody has closed the write end of never sees EOF.
    drop(command);

    let tee = tee(reader, log, context.output);
    let status = child.wait().map_err(|err| err.to_string())?;
    tee.map_err(|err| err.to_string())?;

    Ok(match status.code() {
        Some(0) => ScriptResult::Ok,
        Some(code) => ScriptResult::Failed {
            code: Some(code),
            detail: format!("exited {code}"),
        },
        None => ScriptResult::Failed {
            code: None,
            detail: "was ended by a signal".to_owned(),
        },
    })
}

/// Everything the child inherits, plus the eight variables and the `bin/` entry.
///
/// The caller's environment is inherited **in full** rather than sanitised: a
/// sanitised one would break `SSH_AUTH_SOCK`, `ssh-agent` and every credential
/// helper, and git's credentials are the user's and never bitplane's.
fn environment(command: &mut Command, site: &Site, script: &ScriptName) -> Result<(), String> {
    command
        .env("BITPLANE_SCRIPT", script.as_str())
        // The bare name: the `@` sigil is syntax, and never appears here.
        .env("BITPLANE_PROJECT", site.project.as_str())
        .env("BITPLANE_PROJECT_DIR", &site.project_directory)
        .env("BITPLANE_SOURCE_REPO", &site.source_repo)
        .env("BITPLANE_WORKTREE", &site.worktree)
        .env("BITPLANE_PLANE_DIR", &site.plane_directory)
        .env("BITPLANE_PLANE_ID", &site.plane_id);

    // **Unset** rather than set to a sha when `HEAD` is detached, so a script
    // doing `git switch "$BITPLANE_BRANCH"` fails loudly instead of detaching
    // again. Removed rather than skipped: an outer shell may have one set.
    match Head::read(&site.worktree).as_ref().and_then(Head::branch) {
        Some(branch) => command.env("BITPLANE_BRANCH", branch),
        None => command.env_remove("BITPLANE_BRANCH"),
    };

    command.env("PATH", search_path(&site.project_directory)?);

    Ok(())
}

/// `PATH` with the project's `bin/` in front, so a project can ship its own
/// executables and have them win.
///
/// `bin/` rather than the project directory itself: every file bitplane ever
/// adds there would otherwise become a command-name collision, and `repo.git`
/// holds fetched remote content. A non-existent entry is silently ignored by
/// every shell, so the directory costs nothing until the user makes it.
fn search_path(project_directory: &Path) -> Result<std::ffi::OsString, String> {
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let mut entries = vec![project_directory.join(BIN_DIR_NAME)];
    entries.extend(std::env::split_paths(&inherited));

    // Said out loud rather than shrugged off. The only way this fails is a
    // project directory with a `:` in it, and a script that silently ran
    // without the executables its project ships is the "no trace of why"
    // failure this design refuses everywhere else.
    std::env::join_paths(entries)
        .map_err(|err| format!("{} cannot go on PATH: {err}", project_directory.display()))
}

/// Reads the merged stream to EOF, into the log and out to the sink at once.
fn tee(mut reader: io::PipeReader, log: &mut File, sink: &dyn Fn(&[u8])) -> io::Result<()> {
    let mut buffer = [0u8; 8192];

    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(read) => {
                log.write_all(&buffer[..read])?;
                sink(&buffer[..read]);
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        }
    }
}

/// `<plane-dir>/.bitplane/logs/<YYYYMMDDTHHMMSSZ>-<project>-<script>.log`.
///
/// A compact timestamp because RFC 3339's colons are hostile in filenames. Both
/// names are checked against the plane-id character set before they get here,
/// so neither can reach out of the directory.
fn log_path(site: &Site, script: &ScriptName) -> Result<PathBuf, EngineError> {
    let directory = site.plane_directory.join(BITPLANE_DIR).join(LOGS_DIR);
    fs::create_dir_all(&directory).map_err(|err| EngineError::io(&directory, err))?;

    Ok(directory.join(format!(
        "{}-{}-{script}.log",
        Rfc3339::now().compact(),
        site.project,
    )))
}

/// Where one member's scripts would run, and what it declares.
fn site_of(
    member: &MemberRef,
    worktree: PathBuf,
    plane: InPlane<'_>,
    context: &ScriptContext<'_>,
) -> Result<Option<Site>, EngineError> {
    // An **ad-hoc member** has no `project.toml`, so it has no scripts and
    // behaves exactly like a project with an empty `[scripts]`. So does a
    // `@name` the registry does not hold: an absent file declares nothing,
    // where a file that will not parse is an error and says so.
    let MemberRef::Project(name) = member else {
        return Ok(None);
    };
    let project = ProjectDirectory::of(context.directories, name.clone());
    if !project.is_registered() {
        return Ok(None);
    }
    let file = project.read_project_file()?;

    Ok(Some(Site {
        project: name.clone(),
        source_repo: project_dir::source_repo_of(&project, &file),
        project_directory: project.path().to_path_buf(),
        worktree,
        plane_directory: plane.directory.to_path_buf(),
        plane_id: plane.id.to_owned(),
        scripts: file.scripts,
    }))
}

impl ScriptName {
    /// Checks the character set — the same one a plane id uses, so a name is
    /// never trusted into a path.
    pub fn parse(name: &str) -> Result<ScriptName, InvalidScriptName> {
        if is_well_formed_id(name) {
            Ok(ScriptName(name.to_owned()))
        } else {
            Err(InvalidScriptName)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A `[scripts.<name>]` whose name is not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidScriptName;

impl ScriptPoint {
    /// Both points, in the order `project show` prints them.
    pub const ALL: [ScriptPoint; 2] = [
        ScriptPoint::PostWorktreeCreate,
        ScriptPoint::PreWorktreeRemove,
    ];

    /// The `project.toml` toggle that binds a script to this point.
    pub fn key(self) -> &'static str {
        match self {
            ScriptPoint::PostWorktreeCreate => "post_worktree_create",
            ScriptPoint::PreWorktreeRemove => "pre_worktree_remove",
        }
    }

    /// The point a toggle names, where it names one.
    pub fn named(key: &str) -> Option<ScriptPoint> {
        ScriptPoint::ALL
            .into_iter()
            .find(|point| point.key() == key)
    }
}

impl ScriptCommand {
    /// The program and its arguments, as a child process takes them.
    pub fn invocation(&self) -> (&str, Vec<&str>) {
        match self {
            ScriptCommand::Argv(argv) => (
                argv.first().map(String::as_str).unwrap_or_default(),
                argv.iter().skip(1).map(String::as_str).collect(),
            ),
            ScriptCommand::Shell(line) => (SHELL, vec!["-c", line]),
        }
    }
}

/// Rendered as the one line `bp project show` prints for it.
impl fmt::Display for ScriptCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScriptCommand::Argv(argv) => f.write_str(&argv.join(" ")),
            ScriptCommand::Shell(line) => f.write_str(line),
        }
    }
}

impl fmt::Display for ScriptName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<ScriptName> for String {
    fn from(name: ScriptName) -> String {
        name.0
    }
}

impl TryFrom<String> for ScriptName {
    type Error = InvalidScriptName;

    fn try_from(name: String) -> Result<ScriptName, InvalidScriptName> {
        ScriptName::parse(&name)
    }
}

impl fmt::Display for InvalidScriptName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("is not a valid script name")
    }
}

impl std::error::Error for InvalidScriptName {}

/// How a project renders in a message: `@api`, never a bare name.
pub fn sigilled(project: &str) -> String {
    format!("{PROJECT_SIGIL}{project}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_name_takes_the_same_characters_a_plane_id_does() {
        for name in ["install", "link-alfonz", "db.reset", "a_b", "0"] {
            assert!(ScriptName::parse(name).is_ok(), "{name} should be a name");
        }

        for name in ["", "Install", "-install", "../escape", "a/b", "a b"] {
            assert_eq!(
                ScriptName::parse(name),
                Err(InvalidScriptName),
                "{name} must never reach a path"
            );
        }
    }

    #[test]
    fn an_argv_script_runs_its_first_word_and_a_shell_script_runs_a_shell() {
        let argv = ScriptCommand::Argv(vec![
            "ln".to_owned(),
            "-s".to_owned(),
            "../../.alfonz".to_owned(),
        ]);
        assert_eq!(argv.invocation(), ("ln", vec!["-s", "../../.alfonz"]));
        assert_eq!(argv.to_string(), "ln -s ../../.alfonz");

        let shell = ScriptCommand::Shell("pnpm i && pnpm build".to_owned());
        assert_eq!(
            shell.invocation(),
            (SHELL, vec!["-c", "pnpm i && pnpm build"])
        );
        assert_eq!(shell.to_string(), "pnpm i && pnpm build");
    }

    #[test]
    fn every_point_has_a_toggle_and_every_toggle_names_its_point() {
        for point in ScriptPoint::ALL {
            assert_eq!(ScriptPoint::named(point.key()), Some(point));
        }

        assert_eq!(ScriptPoint::named("post_worktree_created"), None);
    }
}
