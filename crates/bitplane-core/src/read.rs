//! `plane_list`, `plane_show` and `plane_status`: the reads.
//!
//! **None of them takes a lock, writes a file, or repairs anything**, which is
//! what makes them safe to run from a script, from a shell prompt, or in a loop
//! (ADR-0002). That property is checkable by reading [`crate::Reader`] rather
//! than by auditing these bodies — but the bodies are where it is kept true, so
//! nothing here reaches for [`crate::lock`] or [`crate::fsio`].
//!
//! A read costs one `readdir` of the planes directory and one small file per
//! plane. It deliberately does **not** scan a plane directory for worktrees:
//! that scan is affordable in `bp repair`, which is an explicit request, and is
//! what would make membership derived rather than recorded (ADR-0008).
//!
//! Plane resolution lives here rather than in the CLI so it stays executable
//! against a remote host: a client cannot walk a remote filesystem up to
//! `plane.toml` (ADR-0003).

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use crate::directories::Directories;
use crate::error::EngineError;
use crate::head::Head;
use crate::health::{Finding, HealthCheck, PlaneHealth};
use crate::member::{MemberRef, WorktreePath};
use crate::plan;
use crate::plane_dir::{BITPLANE_DIR, LATCH_NAME};
use crate::plane_file::{PLANE_FILE_NAME, PlaneFile};
use crate::repo::Git;
use crate::time::{Rfc3339, ago};
use crate::wire::{
    MemberStatus, MemberView, MemberWork, PlaneList, PlaneListRequest, PlaneRef, PlaneShowRequest,
    PlaneStatus, PlaneStatusRequest, PlaneView,
};

/// Everything a read needs that is not in the request.
pub struct ReadContext<'a> {
    pub directories: &'a Directories,
    pub git: &'a Git,
}

/// Every plane on this host, in directory-name order.
pub fn plane_list(
    request: &PlaneListRequest,
    context: &ReadContext<'_>,
) -> Result<PlaneList, EngineError> {
    let planes = context.directories.planes();
    // The planes directory resolved once, so every plane's own directory is
    // reported the same way whichever command reached it — and so a planes
    // directory configured through a symlink does not make `bp list` and a
    // `bp show` walked up from a worktree disagree.
    let root = canonical(planes);

    let entries = match fs::read_dir(planes) {
        Ok(entries) => entries,
        // No planes directory is no planes, not a failure: nothing has been
        // created here yet, and a read must not create it either.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PlaneList::default());
        }
        Err(err) => return Err(EngineError::io(planes, err)),
    };

    // Only the parent is resolved: the entry keeps the name it has *here*,
    // because that name is the plane's identity, and reporting a symlinked
    // plane directory under its target's name would invent an id mismatch.
    //
    // `readdir` order is whatever the filesystem feels like; a listing a person
    // reads twice has to come back the same way round.
    let mut directories: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .map(|entry| root.join(entry.file_name()))
        .collect();
    directories.sort();

    Ok(PlaneList {
        planes: directories
            .iter()
            // A plane is exactly a directory containing a plane file. Anything
            // else in the planes directory is not a plane and is ignored — an
            // unrelated directory dropped in there is not an error.
            .filter(|directory| directory.join(PLANE_FILE_NAME).is_file())
            .map(|directory| view(directory, request.health, context))
            .collect(),
    })
}

/// One plane, in detail.
pub fn plane_show(
    request: &PlaneShowRequest,
    context: &ReadContext<'_>,
) -> Result<PlaneView, EngineError> {
    let directory = resolve(&request.plane, context.directories)?;
    // Exactly one file to read, so failing to read it is a failure rather than
    // a row — the opposite of `plane_list`, deliberately.
    let file = PlaneFile::read(&directory.join(PLANE_FILE_NAME))?;

    Ok(describe(&file, &directory, request.health, context))
}

/// What git says about every member of one plane.
pub fn plane_status(
    request: &PlaneStatusRequest,
    context: &ReadContext<'_>,
) -> Result<PlaneStatus, EngineError> {
    let directory = resolve(&request.plane, context.directories)?;
    let file = PlaneFile::read(&directory.join(PLANE_FILE_NAME))?;

    let members = file
        .members
        .iter()
        .map(|member| MemberStatus {
            member: member.source.clone(),
            path: member.path.clone(),
            head: Head::read(&member.path.under(&directory)),
            work: interrogate(&member.source, &member.path.under(&directory), context),
        })
        .collect();

    Ok(PlaneStatus {
        id: directory_name(&directory),
        directory: directory.clone(),
        members,
        // `status` has no health flag: the cheap tier costs nothing beside the
        // git invocations it is already making, and it is what gives the
        // command something to exit `3` about.
        health: examine(&file, &directory, HealthCheck::Cheap, context),
    })
}

/// Which directory a [`PlaneRef`] names.
///
/// The path arm canonicalises first — so a worktree reached through a symlink
/// resolves to its real plane — then walks up to the filesystem root, the
/// **innermost** plane file winning, which is the only answer that can be right
/// for nested planes.
///
/// There is **no planes-directory constraint**: a plane found outside the
/// configured planes directory still resolves, because refusing it would make
/// `bp repair` impossible to aim at a plane from inside it (ADR-0008).
///
///
/// Nothing here reads `BITPLANE_PLANE_DIR` or `BITPLANE_PLANE_ID`. Those are
/// written for scripts to consume and never read back: a stale one inherited
/// from an outer shell would report a plane the user is not standing in, and
/// since a plane id is mutable it could resolve to a *different* plane that has
/// since taken the name.
pub fn resolve(plane: &PlaneRef, directories: &Directories) -> Result<PathBuf, EngineError> {
    match plane {
        PlaneRef::Id { id } => {
            let directory = named(directories, id)?;

            if directory.join(PLANE_FILE_NAME).is_file() {
                Ok(directory)
            } else {
                Err(EngineError::PlaneNotFound {
                    sought: plane.clone(),
                })
            }
        }
        PlaneRef::ContainingPath { path } => {
            let from = path.canonicalize().unwrap_or_else(|_| path.clone());
            let mut at = from.as_path();

            loop {
                if at.join(PLANE_FILE_NAME).is_file() {
                    return Ok(at.to_path_buf());
                }
                match at.parent() {
                    Some(parent) => at = parent,
                    None => break,
                }
            }

            Err(EngineError::PlaneNotFound {
                sought: PlaneRef::ContainingPath { path: from },
            })
        }
    }
}

/// The directory a plane id names.
///
/// **Not** [`PlaneId::parse`]: the directory's name *is* the plane's identity,
/// and a directory a user made by hand need not be a well-formed id — refusing
/// to look at one `bp list` happily reports would make it unreachable, and the
/// remedies that say `bp destroy -p <plane>` would name a command that fails.
///
/// What is refused is an id that is not a single path segment, because joining
/// `../…` onto the planes directory would aim every later command outside it.
fn named(directories: &Directories, id: &str) -> Result<PathBuf, EngineError> {
    let mut segments = Path::new(id).components();

    match (segments.next(), segments.next()) {
        (Some(Component::Normal(name)), None) => Ok(canonical(directories.planes()).join(name)),
        _ => Err(EngineError::InvalidPlaneId { id: id.to_owned() }),
    }
}

/// One plane directory, read. A plane file that will not parse becomes a row in
/// an error state rather than stopping the scan.
fn view(directory: &Path, health: HealthCheck, context: &ReadContext<'_>) -> PlaneView {
    match PlaneFile::read(&directory.join(PLANE_FILE_NAME)) {
        Ok(file) => describe(&file, directory, health, context),
        Err(error) => PlaneView {
            id: directory_name(directory),
            directory: directory.to_path_buf(),
            // Not asked for: reading a plane's creation time is part of
            // describing a plane, and there is no plane here — only a directory
            // and an error.
            created_at: None,
            members: Vec::new(),
            health: PlaneHealth {
                checked: health,
                // Reported at every tier, `none` included: this is not a health
                // check but the read's own answer.
                findings: vec![Finding::Unreadable { error }],
            },
        },
    }
}

fn describe(
    file: &PlaneFile,
    directory: &Path,
    health: HealthCheck,
    context: &ReadContext<'_>,
) -> PlaneView {
    PlaneView {
        id: directory_name(directory),
        directory: directory.to_path_buf(),
        created_at: created_at(directory),
        members: file
            .members
            .iter()
            .map(|member| MemberView {
                member: member.source.clone(),
                path: member.path.clone(),
                head: Head::read(&member.path.under(directory)),
            })
            .collect(),
        health: examine(file, directory, health, context),
    }
}

/// The health scan. Findings about the plane come first, then one pass over the
/// members in plane file order.
fn examine(
    file: &PlaneFile,
    directory: &Path,
    check: HealthCheck,
    context: &ReadContext<'_>,
) -> PlaneHealth {
    if check == HealthCheck::None {
        return PlaneHealth::sound(check);
    }

    let name = directory_name(directory);
    let mut findings = Vec::new();

    if file.id.as_str() != name {
        findings.push(Finding::IdMismatch {
            recorded: file.id.clone(),
            directory: name.clone(),
        });
    }

    if let Some(started) = latched_at(directory) {
        findings.push(Finding::CreateNeverCompleted {
            plane: name,
            ago: started.elapsed().ok().map(ago),
            started: Some(Rfc3339::at(started).to_string()),
        });
    }

    for member in &file.members {
        let at = member.path.under(directory);

        if !at.is_dir() {
            findings.push(Finding::MemberWorktreeMissing {
                member: member.source.clone(),
                path: member.path.clone(),
            });
        }

        let source = source_of(&member.source, context.directories);
        if !source.is_dir() {
            findings.push(Finding::SourceRepoMissing {
                member: member.source.clone(),
                source,
            });
            continue;
        }

        // The one finding that costs a git process, which is why `bp list` does
        // not reach it by default.
        if check == HealthCheck::Full && is_prunable(directory, &member.path, &source, context.git)
        {
            findings.push(Finding::Prunable {
                member: member.source.clone(),
                path: member.path.clone(),
            });
        }
    }

    PlaneHealth {
        checked: check,
        findings,
    }
}

/// Asks git about one member's worktree.
fn interrogate(member: &MemberRef, at: &Path, context: &ReadContext<'_>) -> MemberWork {
    if !at.is_dir() {
        return MemberWork::WorktreeMissing;
    }
    // A worktree reads its source repo's config and objects, so git cannot
    // answer for one whose source is gone — and saying so is better than
    // passing git's confusion through.
    if !source_of(member, context.directories).is_dir() {
        return MemberWork::SourceRepoMissing;
    }

    let answered = context
        .git
        .working_tree_changes(at)
        .and_then(|(modified, untracked)| {
            Ok(MemberWork::Reported {
                modified,
                untracked,
                ahead: context.git.commits_not_on_origin(at)?,
            })
        });

    // A member git would not answer for is a row, not a failed read: the other
    // five members still have something to say.
    answered.unwrap_or_else(|error| MemberWork::Unreadable {
        message: error.to_string(),
    })
}

/// Whether git records this worktree at a path that is no longer there.
///
/// The **plane directory** is what gets canonicalised, not the worktree path:
/// git canonicalises at `worktree add` time, so `/tmp/…` comes back as
/// `/private/tmp/…` on macOS, and the worktree this asks about is by definition
/// one that may be gone — canonicalising a path that no longer exists returns
/// it unchanged and the two would never compare equal.
///
/// A source repo git will not answer for is left to `bp doctor`, whose whole
/// job is the source-repo sweep: reporting it here too, in a second vocabulary,
/// would be worse than reporting it once.
fn is_prunable(plane: &Path, path: &WorktreePath, source: &Path, git: &Git) -> bool {
    let wanted = path.under(&canonical(plane));

    git.worktrees(source).is_ok_and(|entries| {
        entries
            .iter()
            .any(|entry| entry.prunable && canonical(&entry.path) == wanted)
    })
}

/// The repository a member is a worktree of.
///
/// An ad-hoc member names its repo outright; a project's is found by reading
/// its `project.toml`. Either way this is the same answer the teardown verbs
/// get, because it is the same function — two ideas of where a member comes
/// from would be two answers.
fn source_of(member: &MemberRef, directories: &Directories) -> PathBuf {
    plan::source_of(member, directories).at
}

/// When the plane directory was made, where the filesystem can say.
///
/// The **directory's** birth time, which is the `mkdir` that claimed it: it
/// survives every rewrite, every worktree add, and a same-filesystem `mv`.
/// `plane.toml`'s own birth time is disqualified — writing by atomic rename
/// swaps the inode, so it is the time of the *last* write (ADR-0008).
fn created_at(directory: &Path) -> Option<String> {
    let born = fs::metadata(directory).ok()?.created().ok()?;

    Some(Rfc3339::at(born).to_string())
}

/// When a still-latched plane was claimed, where there is a latch at all.
///
/// The latch file's own timestamp, taken from the filesystem rather than parsed
/// out of the file: nothing may parse the latch for control flow, so a corrupt
/// or empty one still means "incomplete" (ADR-0004).
fn latched_at(directory: &Path) -> Option<SystemTime> {
    let latch = directory.join(BITPLANE_DIR).join(LATCH_NAME);
    let metadata = fs::metadata(&latch).ok()?;

    metadata.created().or_else(|_| metadata.modified()).ok()
}

/// The directory's name, which **is** the plane's identity.
fn directory_name(directory: &Path) -> String {
    directory
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// A path as the filesystem sees it, or as given where it no longer exists.
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}
