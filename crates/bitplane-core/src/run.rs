//! `plane_scripts`: running a project's declared scripts by name (ADR-0007).
//!
//! ```text
//! 1. resolve the plane, decline a latched one
//! 2. read its membership
//! 3. check every project is a member and every name is declared ← nothing has run
//! 4. run them, in request order, stopping at the first non-zero exit
//! ```
//!
//! **No lock.** A script has no timeout by design, so holding the plane's lock
//! across a `pnpm i && pnpm build` would block every other `bp` aimed at that
//! plane for as long as the build takes — which is why `bp run` is the one
//! mutating verb whose exit codes do not include `5`.
//!
//! **Step 3 finishes before step 4 begins**, for the reason the teardown's
//! refusal pass does: a typo in the fourth script name must not leave three
//! scripts already run.
//!
//! What makes this worth having at all is **reproduction cost**. Re-running a
//! failed script by hand means reconstructing eight `BITPLANE_*` variables, the
//! project's `bin/` entry on `PATH` and the right working directory. That is
//! not something a user gets right, and getting it wrong silently is worse than
//! not retrying at all.

use crate::directories::Directories;
use crate::error::EngineError;
use crate::interrupt::Interrupt;
use crate::plane_dir::OpenPlane;
use crate::read;
use crate::scripts::{self, InPlane, ScriptContext, ScriptName};
use crate::wire::{PlaneScriptsRequest, ScriptsRun};

/// Everything `plane_scripts` needs that is not in the request.
pub struct RunContext<'a> {
    pub directories: &'a Directories,
    pub interrupt: Interrupt,
    /// Where merged script output goes as it arrives.
    pub output: &'a (dyn Fn(&[u8]) + Sync),
}

/// Runs the named scripts, or runs none and says why.
pub fn plane_scripts(
    request: &PlaneScriptsRequest,
    context: &RunContext<'_>,
) -> Result<ScriptsRun, EngineError> {
    let names = validate(request)?;

    let plane = OpenPlane::at(read::resolve(&request.plane, context.directories)?);

    // Same grounds and same remedy as `plane_repair`: a plane `create` claimed
    // and never finished is headed for deletion, so running a project's setup
    // inside one is meaningless work.
    if plane.is_latched() {
        return Err(EngineError::PlaneIncomplete {
            id: plane.id().to_owned(),
        });
    }

    let members = plane
        .read_plane_file()?
        .map(|file| file.members)
        .ok_or_else(|| EngineError::PlaneIncomplete {
            id: plane.id().to_owned(),
        })?;

    let outcomes = scripts::named(
        &request.projects,
        &names,
        &members,
        InPlane {
            id: plane.id(),
            directory: plane.path(),
        },
        &context.scripting(),
    )?;

    if outcomes.iter().any(|outcome| !outcome.succeeded()) {
        return Err(EngineError::ScriptFailed {
            outcomes,
            // There is no such news here: nothing was being built, so the only
            // thing to say is what failed.
            worktree_created: false,
        });
    }

    Ok(ScriptsRun {
        id: plane.id().to_owned(),
        directory: plane.path().to_path_buf(),
        scripts: outcomes,
    })
}

/// What the request asks for that cannot be honoured whatever is on disk.
///
/// Both vectors are non-empty or it is a usage failure: the project is
/// mandatory because nobody gets a six-repo script run by typing nothing, and a
/// run with no script named is a run that would do nothing at all.
///
/// A name outside the plane-id character set is refused here rather than
/// reported as "no such script": a name is never trusted into a path, and the
/// log is named after it.
fn validate(request: &PlaneScriptsRequest) -> Result<Vec<ScriptName>, EngineError> {
    if request.projects.is_empty() || request.names.is_empty() {
        return Err(EngineError::InvalidRequest {
            message: "bp run takes a project and at least one script name".to_owned(),
        });
    }

    request
        .names
        .iter()
        .map(|name| {
            ScriptName::parse(name).map_err(|_| EngineError::InvalidRequest {
                message: format!("{name:?} is not a valid script name"),
            })
        })
        .collect()
}

impl RunContext<'_> {
    /// What one script run needs out of this, and nothing else.
    pub fn scripting(&self) -> ScriptContext<'_> {
        ScriptContext {
            directories: self.directories,
            interrupt: self.interrupt,
            output: self.output,
        }
    }
}
