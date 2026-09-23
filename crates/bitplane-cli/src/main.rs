//! `bp` — the human rendering of the bitplane contract.
//!
//! The CLI owns argument parsing and presentation and nothing else. Every
//! failure it reports is an [`ErrorEnvelope`] built in `bitplane-core`, written
//! to stderr; stdout carries results only (ADR-0003).

mod interrupt;
mod render;

use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;

use bitplane_core::{
    BranchIntent, Directories, DirectoryOverrides, EngineError, ErrorEnvelope, HealthCheck,
    Interrupt, LocalEngine, PlaneAddRequest, PlaneCreateRequest, PlaneDestroyRequest,
    PlaneListRequest, PlaneRef, PlaneRemoveRequest, PlaneScriptsRequest, PlaneShowRequest,
    PlaneStatusRequest, ProjectAddRequest, ProjectFetchRequest, ProjectName, Reason, Request,
    Response, SystemEnvironment, Termination, dispatch,
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use render::Rendering;

fn main() -> std::process::ExitCode {
    interrupt::listen_for_ctrl_c();

    let invocation = run(std::env::args_os());
    report(&invocation, &mut std::io::stdout(), &mut std::io::stderr());

    invocation.termination.exit_code().into()
}

/// Manage planes: named sets of git worktrees that share one lifecycle.
#[derive(Debug, Parser)]
#[command(name = "bp", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    global: GlobalFlags,
}

/// The flags every command accepts. Documented once, on one page, and never
/// repeated per command.
#[derive(Debug, Args)]
struct GlobalFlags {
    /// Where plane directories live for this invocation.
    #[arg(long, global = true, value_name = "path")]
    planes_dir: Option<PathBuf>,

    /// Where project directories live for this invocation.
    #[arg(long, global = true, value_name = "path")]
    projects_dir: Option<PathBuf>,

    /// The config file to read instead of the default.
    #[arg(long, global = true, value_name = "path")]
    config: Option<PathBuf>,

    /// Render machine output instead of human output.
    #[arg(long, global = true)]
    json: bool,
}

/// The verbs. Plane verbs are bare; project verbs are prefixed with `project`.
#[derive(Debug, Subcommand)]
enum Command {
    /// Create a plane and the worktrees of every member named.
    Create(CreateArgs),
    /// Put members into a plane that already exists.
    Add(AddArgs),
    /// List every plane, with its members, their live branches and its health.
    List(ListArgs),
    /// Describe one plane: its members, where they are, and any finding.
    Show(ShowArgs),
    /// Report git's own status across every member of a plane.
    Status(StatusArgs),

    /// Remove every worktree in a plane and the plane directory with them.
    Destroy(DestroyArgs),

    /// Take members out of a plane, leaving the rest of the plane alone.
    Rm(RemoveArgs),

    /// Run a project's declared scripts, by name, on demand.
    Run(RunArgs),

    /// Manage the projects registered on this host.
    #[command(subcommand)]
    Project(ProjectCommand),
}

/// The project verbs. Prefixed with `project`, where a plane verb is bare.
#[derive(Debug, Subcommand)]
enum ProjectCommand {
    /// Register a project from a URL, building the source repo bp will own.
    Add(ProjectAddArgs),

    /// List every project registered on this host.
    List,

    /// Bring owned projects up to date with their forges.
    Fetch(ProjectFetchArgs),
}

/// `bp run @api install`, which reads like `npm run install` on purpose.
///
/// **The project is mandatory**: nobody gets a six-repo script run by typing
/// nothing. It takes script names only and never a point name — points are
/// bindings, not addressable things, so the two never share a namespace.
#[derive(Debug, Args)]
struct RunArgs {
    /// The project whose scripts to run, as `@name` or `name`. Must be a
    /// member of the plane.
    #[arg(value_name = "project")]
    project: String,

    /// The scripts, run in the order you name them.
    #[arg(required = true, value_name = "script")]
    scripts: Vec<String>,

    #[command(flatten)]
    plane: PlaneArgs,
}

#[derive(Debug, Args)]
struct ProjectAddArgs {
    /// Anything git can fetch from: git@…, https://…, ssh://….
    #[arg(value_name = "url")]
    url: String,

    /// The project's name, which is also its directory name.
    #[arg(long, value_name = "name")]
    name: Option<String>,
}

#[derive(Debug, Args)]
struct ProjectFetchArgs {
    /// Which projects to fetch, as `@name` or `name`. With none named, every
    /// registered project is taken in turn.
    #[arg(value_name = "project")]
    projects: Vec<String>,
}

#[derive(Debug, Args)]
struct CreateArgs {
    /// The members: `@name` for a project, a path for an ad-hoc member. Each
    /// may carry a `:branch` suffix.
    #[arg(required = true, value_name = "member")]
    members: Vec<String>,

    /// The branch for every member that does not carry its own.
    #[arg(short, long, value_name = "branch")]
    branch: Option<String>,

    /// The plane id, which is also its directory name.
    #[arg(long, value_name = "id")]
    id: Option<String>,

    #[command(flatten)]
    intent: BranchIntentArgs,

    #[command(flatten)]
    fetch: FetchArg,

    #[command(flatten)]
    scripts: ScriptsArg,
}

/// Whether the branch asked for must exist, must not, or either.
///
/// Neither flag is the default — **resolve**: check the branch out if it is
/// there after the fetch, and create it otherwise.
#[derive(Debug, Args)]
struct BranchIntentArgs {
    /// The branch must not already exist.
    #[arg(long, conflicts_with = "existing_branch")]
    new_branch: bool,

    /// The branch must already exist.
    #[arg(long)]
    existing_branch: bool,
}

/// The fetch every owned project gets before its branch is resolved.
///
/// Switching it off is only honest alongside an explicit branch intent:
/// resolving against a source repo nobody has updated is how a colleague's
/// branch name silently becomes a new, unrelated branch of your own.
#[derive(Debug, Args)]
struct FetchArg {
    /// Do not fetch the owned projects named before building their worktrees.
    #[arg(long)]
    no_fetch: bool,
}

/// The scripts a lifecycle point fires.
///
/// Available on **every** command that runs them, which is the whole answer to
/// *can a `project.toml` make a plane undestroyable*: it cannot, because
/// `bp destroy --no-scripts` always works (ADR-0007).
#[derive(Debug, Args)]
struct ScriptsArg {
    /// Do not run the scripts bound to this command's lifecycle point.
    #[arg(long)]
    no_scripts: bool,
}

impl From<BranchIntentArgs> for BranchIntent {
    fn from(args: BranchIntentArgs) -> BranchIntent {
        match (args.new_branch, args.existing_branch) {
            (true, _) => BranchIntent::RequireNew,
            (_, true) => BranchIntent::RequireExisting,
            _ => BranchIntent::Resolve,
        }
    }
}

/// Where `create` makes a plane, `add` writes into one somebody is already
/// working in — so there is no `--id`, and the plane is named the way every
/// other in-plane verb names one.
#[derive(Debug, Args)]
struct AddArgs {
    /// The members: `@name` for a project, a path for an ad-hoc member. Each
    /// may carry a `:branch` suffix.
    #[arg(required = true, value_name = "member")]
    members: Vec<String>,

    #[command(flatten)]
    plane: PlaneArgs,

    /// The branch for every member that does not carry its own.
    #[arg(short, long, value_name = "branch")]
    branch: Option<String>,

    #[command(flatten)]
    intent: BranchIntentArgs,

    #[command(flatten)]
    fetch: FetchArg,

    #[command(flatten)]
    scripts: ScriptsArg,
}

#[derive(Debug, Args)]
struct ListArgs {
    #[command(flatten)]
    health: HealthArgs,
}

#[derive(Debug, Args)]
struct ShowArgs {
    #[command(flatten)]
    plane: PlaneArgs,

    #[command(flatten)]
    health: HealthArgs,
}

#[derive(Debug, Args)]
struct StatusArgs {
    #[command(flatten)]
    plane: PlaneArgs,
}

#[derive(Debug, Args)]
struct DestroyArgs {
    #[command(flatten)]
    plane: PlaneArgs,

    #[command(flatten)]
    waive: WaiveArg,

    #[command(flatten)]
    scripts: ScriptsArg,
}

#[derive(Debug, Args)]
struct RemoveArgs {
    /// The members to take out: `@name` for a project, a path for an ad-hoc
    /// member. No branch suffix — a member is already on a branch.
    #[arg(required = true, value_name = "member")]
    members: Vec<String>,

    #[command(flatten)]
    plane: PlaneArgs,

    #[command(flatten)]
    waive: WaiveArg,

    #[command(flatten)]
    scripts: ScriptsArg,
}

/// How a command that acts on an existing plane is aimed.
///
/// **A plane is never a positional argument.** Positional slots hold members,
/// branches, script names and project names — things that vary per command — so
/// a plane id can never end up in one by accident.
#[derive(Debug, Args)]
struct PlaneArgs {
    /// The plane. Defaults to the one containing the current directory.
    #[arg(short, long, value_name = "id")]
    plane: Option<String>,
}

#[derive(Debug, Args)]
struct HealthArgs {
    /// How hard to look for findings.
    #[arg(long, value_name = "level", default_value = "cheap")]
    health: HealthLevel,
}

/// The cost tiers, as the command line spells them.
///
/// `cheap` is the default because a listing runs constantly and must not spawn
/// a git per member; `full` is the opt-in that does (ADR-0003).
#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "snake_case")]
enum HealthLevel {
    None,
    Cheap,
    Full,
}

impl From<HealthLevel> for HealthCheck {
    fn from(level: HealthLevel) -> HealthCheck {
        match level {
            HealthLevel::None => HealthCheck::None,
            HealthLevel::Cheap => HealthCheck::Cheap,
            HealthLevel::Full => HealthCheck::Full,
        }
    }
}

/// The waivers, granted per reason and per invocation.
///
/// There is deliberately **no `--force`**: every reason is waived by name, so a
/// waiver can never override the refusal you did not mean to.
#[derive(Debug, Args)]
struct WaiveArg {
    /// Accept one refusal reason, for this invocation only. Repeatable.
    #[arg(long = "waive", value_name = "reason", value_parser = waiver)]
    waive: Vec<Reason>,
}

/// What one invocation amounts to: what it wrote to each stream, and how it
/// ended. Rendered here rather than at the write, so the exit code and the
/// bytes are decided in one place.
struct Invocation {
    stdout: String,
    stderr: String,
    termination: Termination,
}

/// Parses `args` and does what they ask.
fn run<I, T>(args: I) -> Invocation
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();

    match Cli::try_parse_from(args.clone()) {
        Ok(cli) => {
            let rendering = if cli.global.json {
                Rendering::Json
            } else {
                Rendering::Human
            };
            execute(cli, rendering)
        }
        Err(error) => render_parse_outcome(error, rendering_asked_for(&args)),
    }
}

/// How to render a failure clap would not let us parse.
///
/// Read from the raw arguments, because there is no parsed `Cli` to ask — but
/// stopping at `--`, after which every word is a value rather than a flag.
fn rendering_asked_for(args: &[OsString]) -> Rendering {
    if args
        .iter()
        .take_while(|arg| *arg != "--")
        .any(|arg| arg == "--json")
    {
        Rendering::Json
    } else {
        Rendering::Human
    }
}

fn execute(cli: Cli, rendering: Rendering) -> Invocation {
    let Some(command) = cli.command else {
        return failed(no_command(), rendering);
    };

    match answer(command, &cli.global) {
        Ok(response) => {
            // Results and a failure are not alternatives: a fan-out whose rows
            // are the result can still report that it did not fully succeed,
            // and the rows are what says which part did not.
            let termination = termination_for(&response);

            Invocation {
                stdout: render::response(&response, rendering),
                stderr: termination
                    .envelope()
                    .map(|envelope| render::envelope(envelope, rendering))
                    .unwrap_or_default(),
                termination,
            }
        }
        Err(error) => failed(error.envelope(), rendering),
    }
}

fn answer(command: Command, global: &GlobalFlags) -> Result<Response, EngineError> {
    let directories = Directories::resolve(
        &DirectoryOverrides {
            planes_dir: global.planes_dir.clone(),
            projects_dir: global.projects_dir.clone(),
            config: global.config.clone(),
        },
        &SystemEnvironment,
    )?;

    let engine = LocalEngine::new(directories)
        .with_interrupt(Interrupt::process())
        .announcing_lock_waits(|object| {
            // Progress, not a result, so it goes to stderr in both modes.
            let _ = writeln!(std::io::stderr(), "waiting for {}…", object.display());
        })
        .streaming_script_output(|bytes| {
            // Live and merged, so a hang looks like a hang rather than a
            // freeze — and on stderr in both modes, because stdout is the
            // machine contract and a `pnpm` banner inside the JSON breaks it.
            let mut stderr = std::io::stderr();
            let _ = stderr.write_all(bytes);
            let _ = stderr.flush();
        });

    // git is checked once, before any command touches anything (ADR-0001).
    engine.ensure_git_supported()?;

    dispatch(&engine, request_for(command)?)
}

/// A response that reported an interrupt or a finding still goes to stdout —
/// those rows are the repair instruction — so only the exit code says what
/// happened. Drift is a state bitplane reports, not an error.
fn termination_for(response: &Response) -> Termination {
    match response {
        Response::PlaneCreate(created) if created.interrupted => Termination::Interrupted,
        Response::PlaneAdd(added) if added.interrupted => Termination::Interrupted,
        Response::PlaneDestroy(destroyed) if destroyed.interrupted => Termination::Interrupted,
        Response::PlaneRemove(removed) if removed.interrupted => Termination::Interrupted,
        Response::ProjectFetch(fetched) if fetched.interrupted => Termination::Interrupted,
        // A fan-out reports its rows and its failure at once: the rows are the
        // result and go to stdout, and this says the run did not fully succeed.
        Response::ProjectFetch(fetched) => match fetched.failure() {
            Some(error) => Termination::from(error),
            None => Termination::Ok,
        },
        _ if response.has_findings() => Termination::Drift,
        _ => Termination::Ok,
    }
}

/// Clap reports `--help` and `--version` as errors; they are neither.
fn render_parse_outcome(error: clap::Error, rendering: Rendering) -> Invocation {
    use clap::error::ErrorKind;

    match error.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            // clap's own writer: help and version go to stdout, styled or not
            // according to the terminal.
            let _ = error.print();
            Invocation {
                stdout: String::new(),
                stderr: String::new(),
                termination: Termination::Ok,
            }
        }
        ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => failed(no_command(), rendering),
        _ => failed(
            ErrorEnvelope::usage("bad_usage", one_line(&error.render().to_string()))
                .with_remedy("Run `bp --help` to see what bp can do."),
            rendering,
        ),
    }
}

/// Writes what the invocation produced: results on stdout, the envelope on
/// stderr, in both modes.
fn report(invocation: &Invocation, stdout: &mut impl Write, stderr: &mut impl Write) {
    let _ = write!(stdout, "{}", invocation.stdout);
    let _ = write!(stderr, "{}", invocation.stderr);
}

fn failed(envelope: ErrorEnvelope, rendering: Rendering) -> Invocation {
    Invocation {
        stdout: String::new(),
        stderr: render::envelope(&envelope, rendering),
        termination: Termination::Failed(envelope),
    }
}

/// `bp` on its own. Reported the same way whether clap or [`execute`] notices,
/// so the two can never disagree.
fn no_command() -> ErrorEnvelope {
    ErrorEnvelope::usage("no_command", "no command given")
        .with_remedy("Run `bp --help` to see what bp can do.")
}

/// Turns a parsed subcommand into the request that crosses the wire. The
/// `match` is what makes a forgotten wiring a compile error.
fn request_for(command: Command) -> Result<Request, EngineError> {
    Ok(match command {
        Command::List(args) => Request::PlaneList(PlaneListRequest {
            health: args.health.health.into(),
        }),
        Command::Show(args) => Request::PlaneShow(PlaneShowRequest {
            plane: plane_ref(args.plane),
            health: args.health.health.into(),
        }),
        Command::Status(args) => Request::PlaneStatus(PlaneStatusRequest {
            plane: plane_ref(args.plane),
        }),
        Command::Project(ProjectCommand::Add(args)) => Request::ProjectAdd(ProjectAddRequest {
            url: args.url,
            name: args.name,
        }),
        Command::Project(ProjectCommand::List) => Request::ProjectList,
        Command::Project(ProjectCommand::Fetch(args)) => {
            Request::ProjectFetch(ProjectFetchRequest {
                projects: args.projects,
            })
        }
        Command::Destroy(args) => Request::PlaneDestroy(PlaneDestroyRequest {
            plane: plane_ref(args.plane),
            waive: args.waive.waive,
            run_scripts: !args.scripts.no_scripts,
        }),
        Command::Rm(args) => Request::PlaneRemove(PlaneRemoveRequest {
            plane: plane_ref(args.plane),
            members: args.members,
            waive: args.waive.waive,
            run_scripts: !args.scripts.no_scripts,
        }),
        Command::Create(args) => Request::PlaneCreate(PlaneCreateRequest {
            members: args.members,
            branch: args.branch,
            id: args.id,
            intent: args.intent.into(),
            fetch: !args.fetch.no_fetch,
            run_scripts: !args.scripts.no_scripts,
        }),
        Command::Add(args) => Request::PlaneAdd(PlaneAddRequest {
            plane: plane_ref(args.plane),
            members: args.members,
            branch: args.branch,
            intent: args.intent.into(),
            fetch: !args.fetch.no_fetch,
            run_scripts: !args.scripts.no_scripts,
        }),
        Command::Run(args) => Request::PlaneScripts(PlaneScriptsRequest {
            plane: plane_ref(args.plane),
            names: args.scripts,
            // The one slot where the sigil is optional: no path is accepted
            // here, so there is nothing for it to disambiguate.
            projects: vec![project_name(&args.project)?],
        }),
    })
}

/// One `bp run` project argument, with its optional sigil stripped.
fn project_name(value: &str) -> Result<ProjectName, EngineError> {
    ProjectName::parse(value.strip_prefix('@').unwrap_or(value))
}

/// Which plane the command is about.
///
/// Without `--plane` it is the one containing the current directory — carried
/// as a path so the **engine** walks it, which is what keeps the resolution
/// executable against a remote host (ADR-0003).
///
/// `BITPLANE_PLANE_DIR` and `BITPLANE_PLANE_ID` are deliberately not consulted.
/// bitplane writes them for scripts and never reads them back: a stale one
/// inherited from an outer shell would report a plane you are not standing in,
/// and a plane id is mutable, so it could name a different plane entirely.
fn plane_ref(args: PlaneArgs) -> PlaneRef {
    match args.plane {
        Some(id) => PlaneRef::Id { id },
        None => PlaneRef::ContainingPath {
            path: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        },
    }
}

/// One `--waive` value, checked here so a misspelled reason is a usage failure
/// rather than a waiver that silently covers nothing.
fn waiver(value: &str) -> Result<Reason, String> {
    Reason::parse(value).map_err(|_| {
        format!(
            "expected one of {}",
            Reason::ALL
                .iter()
                .map(|reason| reason.tag())
                .collect::<Vec<&str>>()
                .join(", ")
        )
    })
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
        let envelope = envelope_of(run(["bp"]));

        assert_eq!(envelope.error, "no_command");
        assert_eq!(envelope.code, bitplane_core::ExitCode::Usage);
        assert!(envelope.remedy.is_some());
    }

    #[test]
    fn an_unknown_flag_is_a_usage_failure_on_one_line() {
        let envelope = envelope_of(run(["bp", "--nonsense"]));

        assert_eq!(envelope.error, "bad_usage");
        assert_eq!(envelope.code, bitplane_core::ExitCode::Usage);
        assert!(
            !envelope.message.contains('\n'),
            "got: {}",
            envelope.message
        );
    }

    #[test]
    fn create_with_no_member_is_a_usage_failure() {
        let envelope = envelope_of(run(["bp", "create"]));

        assert_eq!(envelope.code, bitplane_core::ExitCode::Usage);
    }

    #[test]
    fn the_two_branch_intents_cannot_both_be_asked_for() {
        let envelope = envelope_of(run([
            "bp",
            "create",
            "/repos/api",
            "-b",
            "feat",
            "--new-branch",
            "--existing-branch",
        ]));

        assert_eq!(envelope.code, bitplane_core::ExitCode::Usage);
    }

    #[test]
    fn a_successful_invocation_writes_nothing_to_stderr() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        report(
            &Invocation {
                stdout: String::new(),
                stderr: String::new(),
                termination: Termination::Ok,
            },
            &mut stdout,
            &mut stderr,
        );

        assert!(stderr.is_empty());
    }

    #[test]
    fn a_failure_is_rendered_for_a_human_by_default() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        report(&run(["bp"]), &mut stdout, &mut stderr);

        let written = String::from_utf8(stderr).unwrap();
        assert!(written.starts_with("error[no_command]: "), "got {written}");
        assert!(stdout.is_empty(), "stdout carries results only");
    }

    #[test]
    fn a_failure_is_one_line_of_json_under_json() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        report(&run(["bp", "--json"]), &mut stdout, &mut stderr);

        let written = String::from_utf8(stderr).unwrap();
        assert_eq!(written.lines().count(), 1);
        assert!(serde_json::from_str::<serde_json::Value>(&written).is_ok());
        assert!(stdout.is_empty(), "stdout carries results only");
    }

    fn envelope_of(invocation: Invocation) -> ErrorEnvelope {
        invocation
            .termination
            .envelope()
            .expect("a failure")
            .clone()
    }
}
