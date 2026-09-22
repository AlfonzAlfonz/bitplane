//! The error envelope, and the typed failures that produce it.
//!
//! The envelope is constructed here, in the core crate, so the JSON written to
//! stderr is byte-identical across the CLI, a future MCP surface and the SSH
//! servant (ADR-0003). stdout carries results only.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::exit::ExitCode;
use crate::git::{GitVersion, MINIMUM_GIT_VERSION};
use crate::member::PROJECT_SIGIL;
use crate::outcome::{Outcome, PerMember};
use crate::plane_id::GENERATED_ID_PREFIX;
use crate::refusal::{Refusal, waive_flags};
use crate::wire::{CreatedMember, PlaneRef, RemovedMember};

/// What every bitplane failure looks like on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    /// A stable machine tag — matched on by scripts, never rendered to a user.
    pub error: String,
    /// The code the process exits with.
    pub code: ExitCode,
    /// One human sentence saying what went wrong.
    pub message: String,
    /// The individual things that were wrong, where the failure has more than
    /// one. A refusal across four members is four problems.
    pub problems: Vec<Problem>,
    /// What to do about it, where bitplane knows.
    pub remedy: Option<String>,
}

/// One of the things that was wrong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    /// What the problem is about — a member, a plane id, a path — rendered for
    /// display. `None` where the problem is about the operation as a whole.
    pub subject: Option<String>,
    /// One human sentence about this subject.
    pub message: String,
}

/// Everything that can go wrong inside the engine.
///
/// Serialisable in both directions, because a fan-out row carries one of these
/// as data rather than as an envelope (ADR-0003).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
#[non_exhaustive]
pub enum EngineError {
    /// No `git` on `PATH`.
    GitMissing,
    /// `git` is on `PATH` but would not run.
    GitUnusable { message: String },
    /// `git --version` printed something that is not a version.
    GitVersionUnreadable { reported: String },
    /// `git` is older than bitplane's floor (ADR-0001).
    GitTooOld {
        found: GitVersion,
        required: GitVersion,
    },
    /// The request could not be acted on as given.
    InvalidRequest { message: String },
    /// A user-chosen plane id starting with the reserved generated-id prefix.
    ReservedPlaneId,
    /// A plane id outside `[a-z0-9][a-z0-9._-]*`, 64 characters at most.
    InvalidPlaneId { id: String },
    /// The id names something that is already there. Classified, so the remedy
    /// names what is actually at the path (ADR-0004).
    PlaneIdInUse { id: String, found: Occupant },
    /// A string that is not a project name, where one was required.
    InvalidProjectName { name: String },
    /// A member's derived worktree path would start with `.bitplane`.
    ReservedPathSegment { path: String },
    /// Two members of one plane resolving to the same thing. A plane holds at
    /// most one worktree per project.
    ///
    /// `plane` names the plane the member is already in, where there is one;
    /// `create` has not claimed a plane yet, so for it the member was simply
    /// named twice.
    DuplicateMember {
        member: String,
        plane: Option<String>,
    },
    /// Two members that are different paths but one repository — a checkout
    /// and a linked worktree of it, say.
    SameRepository { first: String, second: String },
    /// Two different members whose worktrees would land at the same place.
    /// Plane layout is a pure function of the source, so there is no
    /// disambiguator to reach for.
    MemberPathCollision {
        first: String,
        second: String,
        path: String,
    },
    /// A name that is not a project on this host.
    ProjectNotFound { name: String },
    /// Nothing at the ref given is a plane. Two different sentences, because
    /// "you are not standing in one" and "there is no plane by that name" have
    /// different ways out.
    PlaneNotFound { sought: PlaneRef },
    /// The name a new project would take is already a project.
    ///
    /// Refused with a suggestion rather than silently disambiguated: a
    /// generated default that renames itself behind the user's back is worse
    /// than an error naming the conflict. `suggestion` is absent where the user
    /// chose the name themselves — there is nothing to suggest when the name
    /// was not derived.
    ProjectNameTaken {
        name: String,
        suggestion: Option<String>,
    },
    /// The name derived from a source is not a project name.
    ///
    /// Distinct from [`EngineError::InvalidProjectName`] because the user did
    /// not type this one — the URL did — so the remedy names the flag that
    /// overrides it rather than reciting the charset at someone who never
    /// chose a name.
    DerivedNameInvalid {
        derived: String,
        suggestion: Option<String>,
    },
    /// A `project_add` that did not finish. **The registration is unwound and
    /// the object store is kept**: `init --bare` + `fetch` is resumable in a
    /// way `clone` is not, so a partial object store is reused by the next
    /// attempt (ADR-0005). No `project.toml` was written, so by the existing
    /// rule the leftover directory simply is not a project.
    ProjectAddAborted {
        name: String,
        /// The object store this run kept, which a retry will reuse. `None`
        /// where the run gave up before there was one to keep.
        kept: Option<PathBuf>,
        /// Which step gave up, and what git said about it.
        problem: Problem,
    },
    /// At least one project could not be fetched. The per-project rows are the
    /// result and are reported alongside this, because a fan-out row is data
    /// rather than an error once the fan-out has begun.
    FetchFailed {
        failed: usize,
        /// The projects there was something to fetch **for**. An adopted
        /// project is a row, never a denominator.
        fetchable: usize,
        problems: Vec<Problem>,
    },
    /// A member with no branch, from a suffix or from `-b`.
    BranchUnspecified { member: String },
    /// A member written so that the branch suffix cannot be told from the path.
    MemberPathAmbiguous { spec: String },
    /// An ad-hoc member pointing at something that is not a git repository.
    MemberNotARepository { path: PathBuf, reason: String },
    /// `--new-branch` on a branch that exists, or `--existing-branch` on one
    /// that does not.
    BranchIntentUnmet {
        member: String,
        branch: String,
        wanted: BranchWanted,
    },
    /// A new branch was asked for and there is nothing to cut it from.
    BaseBranchUnresolved { member: String, branch: String },
    /// The branch is checked out in some worktree of the member's own repo, so
    /// git will not have it. Reported in bitplane's words, not git's.
    BranchOccupied {
        branch: String,
        repo: PathBuf,
        worktree: PathBuf,
        /// Whether the holding worktree's directory is gone. Git refuses all
        /// the same, but the way out is to clear the record rather than to
        /// check something else out.
        stale: bool,
    },
    /// A file bitplane needs did not parse.
    ParseError {
        path: PathBuf,
        message: String,
        /// What was legal there, for the remedy. Empty where the failure is not
        /// about a key.
        legal_keys: Vec<String>,
    },
    /// A git command bitplane ran did not succeed.
    GitFailed { message: String },
    /// `create` produced no plane. The per-member rows ride inside the error,
    /// because the envelope `Err` is for operations that produced no durable
    /// state — "never started" and "started, then fully unwound" alike
    /// (ADR-0004).
    CreateAborted {
        id: String,
        members: Vec<PerMember<CreatedMember>>,
        rollback: Vec<PerMember<()>>,
        /// Whether the unwind itself failed, leaving a directory behind.
        remnant: bool,
    },
    /// Work that would be lost. **Every** reason across **every** member, in
    /// one error, because a half-destroyed plane is worse than a refused one —
    /// so one re-run with the right waivers finishes the job.
    Refused {
        plane: String,
        operation: Teardown,
        /// How many members raised at least one surviving reason.
        blocking: usize,
        /// How many members were under consideration — the plane's, for
        /// `destroy`; the ones named, for `rm`.
        considered: usize,
        refusals: Vec<Refusal>,
    },
    /// A plane `create` claimed and never finished. Acting on one is meaningless
    /// work on a thing headed for deletion, so every verb but `destroy` declines
    /// and points at the one that clears it.
    PlaneIncomplete { id: String },
    /// A member that is not in this plane.
    ProjectNotInPlane { member: String, plane: String },
    /// Worktrees this run could not remove. The rows ride inside the error,
    /// because `destroy` converges: they are what a re-run still has to do.
    RemovalFailed {
        plane: String,
        operation: Teardown,
        members: Vec<PerMember<RemovedMember>>,
    },
    /// A lock could not be taken in time. The one failure that means "try
    /// again" rather than "this did not work".
    LockTimeout { object: PathBuf },
    /// The filesystem refused.
    Io { path: PathBuf, message: String },
}

/// What `create` found at a plane directory it could not claim.
///
/// Three different things, so the remedy names what is actually there rather
/// than saying "already in use" and leaving the user to look.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Occupant {
    /// A plane, complete.
    Plane,
    /// A plane directory whose `create` never finished. Nothing in it is the
    /// user's, so it is safe to discard.
    LatchedRemnant,
    /// A claimed directory with no plane file — the window between `mkdir` and
    /// the first write, or what a crash in it left.
    ClaimWithoutPlaneFile,
}

/// Which of the two teardown verbs is speaking.
///
/// They carry identical rules — removing a member destroys exactly as much work
/// as destroying a one-member plane (ADR-0006) — so they share every failure and
/// differ only in the sentence they print.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Teardown {
    /// `bp destroy`: the whole plane.
    Destroy,
    /// `bp rm`: the members named.
    Remove,
}

/// Which way a branch intent was not met.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BranchWanted {
    /// `--new-branch`, and the branch is already there.
    New,
    /// `--existing-branch`, and it is not.
    Existing,
}

impl EngineError {
    /// The stable machine tag for this failure.
    pub fn tag(&self) -> &'static str {
        match self {
            EngineError::GitMissing => "git_missing",
            EngineError::GitUnusable { .. } => "git_unusable",
            EngineError::GitVersionUnreadable { .. } => "git_version_unreadable",
            EngineError::GitTooOld { .. } => "git_too_old",
            EngineError::InvalidRequest { .. } => "invalid_request",
            EngineError::ReservedPlaneId => "reserved_plane_id",
            EngineError::InvalidPlaneId { .. } => "invalid_plane_id",
            EngineError::PlaneIdInUse { .. } => "plane_id_in_use",
            EngineError::InvalidProjectName { .. } => "invalid_project_name",
            EngineError::ReservedPathSegment { .. } => "reserved_path_segment",
            EngineError::DuplicateMember { .. } => "duplicate_member",
            EngineError::SameRepository { .. } => "same_repository",
            EngineError::MemberPathCollision { .. } => "member_path_collision",
            EngineError::ProjectNotFound { .. } => "project_not_found",
            EngineError::PlaneNotFound { .. } => "plane_not_found",
            EngineError::ProjectNameTaken { .. } => "project_name_taken",
            EngineError::DerivedNameInvalid { .. } => "derived_name_invalid",
            EngineError::ProjectAddAborted { .. } => "project_add_aborted",
            EngineError::FetchFailed { .. } => "fetch_failed",
            EngineError::BranchUnspecified { .. } => "branch_unspecified",
            EngineError::MemberPathAmbiguous { .. } => "member_path_ambiguous",
            EngineError::MemberNotARepository { .. } => "member_not_a_repository",
            EngineError::BranchIntentUnmet { .. } => "branch_intent_unmet",
            EngineError::BaseBranchUnresolved { .. } => "base_branch_unresolved",
            EngineError::BranchOccupied { .. } => "branch_occupied",
            EngineError::ParseError { .. } => "parse_error",
            EngineError::GitFailed { .. } => "git_failed",
            EngineError::CreateAborted { .. } => "create_aborted",
            EngineError::Refused { .. } => "refused",
            EngineError::PlaneIncomplete { .. } => "plane_incomplete",
            EngineError::ProjectNotInPlane { .. } => "project_not_in_plane",
            EngineError::RemovalFailed { .. } => "removal_failed",
            EngineError::LockTimeout { .. } => "lock_timeout",
            EngineError::Io { .. } => "io",
        }
    }

    /// The code the process exits with when this failure is what stopped it.
    pub fn exit_code(&self) -> ExitCode {
        match self {
            EngineError::GitMissing
            | EngineError::GitUnusable { .. }
            | EngineError::GitVersionUnreadable { .. }
            | EngineError::GitTooOld { .. } => ExitCode::PrerequisiteMissing,

            // Usage: the request was malformed, or what it asked to create
            // already exists. Re-running it identically will never help.
            EngineError::InvalidRequest { .. }
            | EngineError::ReservedPlaneId
            | EngineError::InvalidPlaneId { .. }
            | EngineError::PlaneIdInUse { .. }
            | EngineError::InvalidProjectName { .. }
            | EngineError::ReservedPathSegment { .. }
            | EngineError::DuplicateMember { .. }
            | EngineError::SameRepository { .. }
            | EngineError::MemberPathCollision { .. }
            | EngineError::ProjectNotFound { .. }
            | EngineError::PlaneNotFound { .. }
            | EngineError::ProjectNameTaken { .. }
            | EngineError::DerivedNameInvalid { .. }
            | EngineError::BranchUnspecified { .. }
            | EngineError::MemberPathAmbiguous { .. }
            | EngineError::MemberNotARepository { .. }
            | EngineError::BranchIntentUnmet { .. }
            | EngineError::BaseBranchUnresolved { .. }
            | EngineError::ProjectNotInPlane { .. } => ExitCode::Usage,

            EngineError::LockTimeout { .. } => ExitCode::Busy,

            // Failure: the operation ran and did not succeed.
            EngineError::BranchOccupied { .. }
            | EngineError::ParseError { .. }
            | EngineError::GitFailed { .. }
            | EngineError::CreateAborted { .. }
            | EngineError::ProjectAddAborted { .. }
            | EngineError::FetchFailed { .. }
            | EngineError::Refused { .. }
            | EngineError::PlaneIncomplete { .. }
            | EngineError::RemovalFailed { .. }
            | EngineError::Io { .. } => ExitCode::Failure,
        }
    }

    /// Renders this failure as the envelope written to stderr.
    pub fn envelope(&self) -> ErrorEnvelope {
        ErrorEnvelope {
            error: self.tag().to_owned(),
            code: self.exit_code(),
            message: self.to_string(),
            problems: self.problems(),
            remedy: self.remedy(),
        }
    }

    fn problems(&self) -> Vec<Problem> {
        match self {
            EngineError::LockTimeout { object } => {
                vec![Problem::about_path(object, "is locked by another process")]
            }
            EngineError::Io { path, message } => vec![Problem::about_path(path, message)],
            EngineError::BranchOccupied {
                branch,
                worktree,
                stale,
                ..
            } => vec![Problem::about_path(
                worktree,
                if *stale {
                    "is gone, but git still records it as holding the branch".to_owned()
                } else {
                    format!("its working tree is on {branch}")
                },
            )],
            // The rows a failed `create` carries are the repair instruction, so
            // they are the problems: what failed, and what was not attempted
            // because of it. A member that succeeded is not a problem — it was
            // unwound with the rest.
            EngineError::CreateAborted {
                members, rollback, ..
            } => members
                .iter()
                .filter_map(problem_from_row)
                .chain(rollback.iter().filter_map(problem_from_rollback))
                .collect(),
            EngineError::ProjectAddAborted { problem, .. } => vec![problem.clone()],
            EngineError::FetchFailed { problems, .. } => problems.clone(),
            // Each reason is its own problem, so a member with three of them
            // prints three lines and the remedy can name all three.
            EngineError::Refused { refusals, .. } => refusals
                .iter()
                .map(|refusal| Problem::about(refusal.member.to_string(), &refusal.detail))
                .collect(),
            EngineError::RemovalFailed { members, .. } => {
                members.iter().filter_map(problem_from_removal).collect()
            }
            _ => Vec::new(),
        }
    }

    fn remedy(&self) -> Option<String> {
        match self {
            EngineError::GitMissing => Some(format!(
                "Install git {MINIMUM_GIT_VERSION} or newer and put it on PATH."
            )),
            EngineError::GitUnusable { .. } | EngineError::GitVersionUnreadable { .. } => Some(
                format!("Check that `git --version` reports git {MINIMUM_GIT_VERSION} or newer."),
            ),
            EngineError::GitTooOld { required, .. } => {
                Some(format!("Upgrade git to {required} or newer."))
            }
            EngineError::LockTimeout { .. } => {
                Some("Another bitplane process holds it; retry once that one finishes.".to_owned())
            }
            EngineError::ReservedPlaneId => Some(format!(
                "Choose an id that does not start with {GENERATED_ID_PREFIX}."
            )),
            EngineError::InvalidPlaneId { .. } => Some(
                "Use lowercase letters, digits and . _ - ; start with a letter or digit; \
                 64 characters at most."
                    .to_owned(),
            ),
            EngineError::PlaneIdInUse { id, found } => Some(match found {
                Occupant::Plane => {
                    "Choose another id, or destroy the existing plane first.".to_owned()
                }
                Occupant::LatchedRemnant => {
                    format!("Nothing in it is yours; run bp destroy -p {id} to clear it.")
                }
                Occupant::ClaimWithoutPlaneFile => {
                    "Run bp doctor to see what is in it, then remove the directory by hand."
                        .to_owned()
                }
            }),
            EngineError::InvalidProjectName { .. } => Some(
                "Use lowercase letters, digits and . _ - ; start with a letter or digit."
                    .to_owned(),
            ),
            EngineError::ReservedPathSegment { .. } => {
                Some("Move the repository out of a directory called .bitplane.".to_owned())
            }
            EngineError::DuplicateMember { member, plane } => Some(match plane {
                Some(_) => format!("Run bp rm {member}, then bp add {member}:<branch>."),
                None => {
                    "A plane holds at most one worktree per repository; name it once.".to_owned()
                }
            }),
            EngineError::SameRepository { .. } => Some(
                "A plane holds at most one worktree per repository; name one of them.".to_owned(),
            ),
            EngineError::MemberPathCollision { .. } => Some(
                "A plane holds one worktree per derived path; put one of them in another plane."
                    .to_owned(),
            ),
            EngineError::ProjectNotFound { .. } => {
                Some("Run bp project list to see what projects exist.".to_owned())
            }
            EngineError::PlaneNotFound { sought } => Some(match sought {
                PlaneRef::Id { .. } => "Run bp list to see what planes exist.".to_owned(),
                PlaneRef::ContainingPath { .. } => {
                    "cd into a plane, or name one with --plane.".to_owned()
                }
            }),
            EngineError::ProjectNameTaken { suggestion, .. } => Some(match suggestion {
                Some(free) => format!("{free} is free; re-run with --name {free}."),
                None => "Choose another name, or remove the project holding it.".to_owned(),
            }),
            EngineError::DerivedNameInvalid { suggestion, .. } => Some(match suggestion {
                Some(free) => format!("Re-run with --name {free}."),
                None => "Re-run with --name <name>, using lowercase letters, digits \
                         and . _ - ."
                    .to_owned(),
            }),
            EngineError::ProjectAddAborted { kept, .. } => Some(match kept {
                Some(kept) => format!(
                    "The objects fetched so far were kept at {}; \
                     re-running bp project add will reuse them.",
                    kept.display()
                ),
                None => "Nothing was kept. Fix what the row reports, then run \
                         bp project add again."
                    .to_owned(),
            }),
            // None. The rows name every project that failed and why, and a
            // single sentence cannot answer four different forges at once.
            EngineError::FetchFailed { .. } => None,
            EngineError::BranchUnspecified { member } => Some(format!(
                "Pass -b <branch>, or write the member as {member}:<branch>."
            )),
            EngineError::MemberPathAmbiguous { .. } => {
                Some("Register it with bp project adopt and use its @name instead.".to_owned())
            }
            EngineError::MemberNotARepository { .. } => Some(
                "Point at a directory that is a git repository, or create one with git init."
                    .to_owned(),
            ),
            EngineError::BranchIntentUnmet { wanted, .. } => Some(match wanted {
                BranchWanted::New => {
                    "Drop --new-branch, or choose a name no branch has taken.".to_owned()
                }
                BranchWanted::Existing => {
                    "Drop --existing-branch, or create the branch first.".to_owned()
                }
            }),
            EngineError::BaseBranchUnresolved { .. } => Some(
                "Set one with git remote set-head origin <branch>, or pass --existing-branch \
                 to use a branch that is already there."
                    .to_owned(),
            ),
            EngineError::BranchOccupied { repo, stale, .. } => Some(if *stale {
                format!(
                    "Run git worktree prune in {} to clear the stale record, then try again.",
                    repo.display()
                )
            } else {
                "Check out a different branch there, or give this member a branch no worktree \
                 holds."
                    .to_owned()
            }),
            EngineError::ParseError { legal_keys, .. } => {
                (!legal_keys.is_empty()).then(|| format!("Legal keys are {}.", listed(legal_keys)))
            }
            EngineError::CreateAborted { id, remnant, .. } => Some(if *remnant {
                format!("Run bp destroy -p {id} to clear the remnant.")
            } else {
                "Nothing was left behind. Fix what the rows report, then run bp create again."
                    .to_owned()
            }),
            EngineError::Refused { refusals, .. } => Some(format!(
                "Inspect the members listed. Re-run with {} to accept losing that work.",
                waive_flags(&refusals.iter().collect::<Vec<&Refusal>>())
            )),
            EngineError::PlaneIncomplete { id } => Some(format!(
                "Nothing in it is yours; run bp destroy -p {id} to clear it."
            )),
            EngineError::ProjectNotInPlane { .. } => {
                Some("Run bp show to see the members of this plane.".to_owned())
            }
            EngineError::RemovalFailed { operation, .. } => Some(format!(
                "Fix what the rows report, then run bp {} again; it converges, \
                 so re-running only finishes what is left.",
                operation.verb()
            )),
            EngineError::InvalidRequest { .. }
            | EngineError::GitFailed { .. }
            | EngineError::Io { .. } => None,
        }
    }
}

impl Teardown {
    /// The verb a user types, for a remedy that can be pasted.
    pub fn verb(self) -> &'static str {
        match self {
            Teardown::Destroy => "destroy",
            Teardown::Remove => "rm",
        }
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::GitMissing => write!(
                f,
                "git was not found on PATH; bitplane requires git {MINIMUM_GIT_VERSION} or newer"
            ),
            EngineError::GitUnusable { message } => write!(
                f,
                "git could not be run ({message}); bitplane requires git {MINIMUM_GIT_VERSION} or newer"
            ),
            EngineError::GitVersionUnreadable { reported } => write!(
                f,
                "git's version could not be read from {reported:?}; bitplane requires git {MINIMUM_GIT_VERSION} or newer"
            ),
            EngineError::GitTooOld { found, required } => {
                write!(
                    f,
                    "git {found} is too old; bitplane requires git {required} or newer"
                )
            }
            EngineError::InvalidRequest { message } => write!(f, "{message}"),
            EngineError::ReservedPlaneId => {
                write!(
                    f,
                    "{GENERATED_ID_PREFIX} is reserved for generated plane ids"
                )
            }
            EngineError::InvalidPlaneId { id } => write!(f, "{id} is not a valid plane id"),
            EngineError::PlaneIdInUse { id, found } => match found {
                Occupant::Plane => write!(f, "{id} is already a plane"),
                Occupant::LatchedRemnant => {
                    write!(f, "{id} exists but was never completed")
                }
                Occupant::ClaimWithoutPlaneFile => {
                    write!(f, "{id} is a claimed directory with no plane file")
                }
            },
            EngineError::InvalidProjectName { name } => {
                write!(f, "{name} is not a valid project name")
            }
            EngineError::ReservedPathSegment { path } => write!(
                f,
                "a worktree of this repo would land at {path}, which is reserved"
            ),
            EngineError::DuplicateMember { member, plane } => match plane {
                Some(plane) => write!(f, "{member} is already a member of {plane}"),
                None => write!(f, "{member} is named twice"),
            },
            EngineError::SameRepository { first, second } => write!(
                f,
                "{second} is a worktree of the same repository as {first}"
            ),
            EngineError::MemberPathCollision {
                first,
                second,
                path,
            } => write!(f, "{first} and {second} would both land at {path}"),
            EngineError::ProjectNotFound { name } => {
                write!(f, "there is no project called {name}")
            }
            EngineError::PlaneNotFound { sought } => match sought {
                PlaneRef::Id { id } => write!(f, "there is no plane called {id}"),
                PlaneRef::ContainingPath { path } => {
                    write!(f, "no plane contains {}", path.display())
                }
            },
            EngineError::ProjectNameTaken { name, .. } => {
                write!(f, "{name} is already a project")
            }
            EngineError::DerivedNameInvalid { derived, .. } => {
                write!(
                    f,
                    "{derived} is not a name bitplane can derive a project from"
                )
            }
            EngineError::ProjectAddAborted { name, .. } => {
                write!(f, "{PROJECT_SIGIL}{name} was not registered")
            }
            EngineError::FetchFailed {
                failed, fetchable, ..
            } => write!(
                f,
                "{failed} of {fetchable} fetchable projects could not be fetched"
            ),
            EngineError::BranchUnspecified { member } => {
                write!(f, "no branch given for {member}")
            }
            EngineError::MemberPathAmbiguous { spec } => {
                write!(f, "{spec} could be a path or a member with a branch suffix")
            }
            EngineError::MemberNotARepository { path, reason } => {
                write!(f, "{} {reason}", path.display())
            }
            EngineError::BranchIntentUnmet {
                member,
                branch,
                wanted,
            } => match wanted {
                BranchWanted::New => write!(f, "{branch} already exists in {member}"),
                BranchWanted::Existing => write!(f, "{branch} does not exist in {member}"),
            },
            EngineError::BaseBranchUnresolved { member, branch } => {
                write!(f, "{member} has no default branch to cut {branch} from")
            }
            EngineError::BranchOccupied {
                branch, worktree, ..
            } => write!(
                f,
                "{branch} is already checked out in {}",
                worktree.display()
            ),
            EngineError::ParseError { path, message, .. } => {
                write!(f, "{}: {message}", path.display())
            }
            EngineError::GitFailed { message } => f.write_str(message),
            EngineError::CreateAborted { id, remnant, .. } => {
                if *remnant {
                    write!(
                        f,
                        "create did not finish and {id} could not be fully removed"
                    )
                } else {
                    write!(f, "create did not finish; {id} was removed")
                }
            }
            EngineError::Refused {
                plane,
                operation,
                blocking,
                considered,
                ..
            } => match operation {
                Teardown::Destroy => write!(
                    f,
                    "refusing to destroy {plane}: {blocking} of {considered} members have work \
                     that would be lost"
                ),
                Teardown::Remove => write!(
                    f,
                    "refusing to remove {blocking} of {considered} members from {plane}: they \
                     have work that would be lost"
                ),
            },
            EngineError::PlaneIncomplete { id } => {
                write!(f, "{id} was never finished being created")
            }
            EngineError::ProjectNotInPlane { member, plane } => {
                write!(f, "{member} is not a member of {plane}")
            }
            EngineError::RemovalFailed { plane, members, .. } => write!(
                f,
                "{} of {} worktrees could not be removed from {plane}",
                members.iter().filter(|row| row.is_failure()).count(),
                members.len(),
            ),
            EngineError::LockTimeout { object } => {
                write!(f, "timed out waiting for the lock on {}", object.display())
            }
            EngineError::Io { path, message } => {
                write!(f, "{}: {message}", path.display())
            }
        }
    }
}

impl Error for EngineError {}

impl EngineError {
    /// The filesystem refusing, named by the path it refused.
    pub fn io(path: &Path, err: std::io::Error) -> EngineError {
        EngineError::Io {
            path: path.to_path_buf(),
            message: err.to_string(),
        }
    }
}

/// One failed member of an aborted `create`, as a problem.
fn problem_from_row(row: &PerMember<CreatedMember>) -> Option<Problem> {
    let message = match &row.outcome {
        Outcome::Failed(error) => format!("failed: {error}"),
        Outcome::Skipped(reason) => format!("skipped: {}", reason.reason()),
        Outcome::Ok(_) | Outcome::AlreadyDone => return None,
    };

    Some(Problem::about(row.member.to_string(), message))
}

/// A member whose worktree would not go.
fn problem_from_removal(row: &PerMember<RemovedMember>) -> Option<Problem> {
    match &row.outcome {
        Outcome::Failed(error) => Some(Problem::about(
            row.member.to_string(),
            format!("could not be removed: {error}"),
        )),
        _ => None,
    }
}

/// A member whose worktree the unwind could not take back. Only the failures:
/// a clean rollback is what the user expects and needs no line.
fn problem_from_rollback(row: &PerMember<()>) -> Option<Problem> {
    match &row.outcome {
        Outcome::Failed(error) => Some(Problem::about(
            row.member.to_string(),
            format!("could not be removed: {error}"),
        )),
        _ => None,
    }
}

/// `a`, `a and b`, `a, b and c` — the form the remedies are written in.
fn listed(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

impl ErrorEnvelope {
    /// A usage failure — a malformed command line, or a name already taken.
    pub fn usage(error: impl Into<String>, message: impl Into<String>) -> ErrorEnvelope {
        ErrorEnvelope {
            error: error.into(),
            code: ExitCode::Usage,
            message: message.into(),
            problems: Vec::new(),
            remedy: None,
        }
    }

    /// Attaches the individual things that were wrong.
    pub fn with_problems(mut self, problems: Vec<Problem>) -> ErrorEnvelope {
        self.problems = problems;
        self
    }

    /// Attaches what to do about it.
    pub fn with_remedy(mut self, remedy: impl Into<String>) -> ErrorEnvelope {
        self.remedy = Some(remedy.into());
        self
    }

    /// The single JSON line written to stderr.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("an error envelope is always serialisable")
    }
}

impl Problem {
    /// A problem with the operation as a whole.
    pub fn new(message: impl Into<String>) -> Problem {
        Problem {
            subject: None,
            message: message.into(),
        }
    }

    /// A problem with one named thing.
    pub fn about(subject: impl Into<String>, message: impl Into<String>) -> Problem {
        Problem {
            subject: Some(subject.into()),
            message: message.into(),
        }
    }

    /// A problem with one path. The path is rendered only here, at the wire
    /// boundary; it is held as a path type everywhere else (ADR-0001).
    pub fn about_path(path: &Path, message: impl Into<String>) -> Problem {
        Problem::about(path.display().to_string(), message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_exit_code_but_ok_and_drift_is_reachable_from_a_typed_failure() {
        let cases = [
            (EngineError::GitMissing, ExitCode::PrerequisiteMissing),
            (
                EngineError::InvalidRequest {
                    message: "nope".to_owned(),
                },
                ExitCode::Usage,
            ),
            (
                EngineError::LockTimeout {
                    object: PathBuf::from("/planes/bp-a3f9c2e1/.bitplane/lock"),
                },
                ExitCode::Busy,
            ),
            (
                EngineError::Io {
                    path: PathBuf::from("/planes/bp-a3f9c2e1/plane.toml"),
                    message: "permission denied".to_owned(),
                },
                ExitCode::Failure,
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.exit_code(), expected, "for {error:?}");
            assert_eq!(error.envelope().code, expected, "for {error:?}");
        }
    }

    #[test]
    fn the_envelope_is_the_five_contract_fields_in_order() {
        // Asserted on the exact bytes: the envelope has to serialise identically
        // across the CLI, an MCP surface and the SSH servant (ADR-0003).
        let envelope = ErrorEnvelope::usage("no_command", "no command given")
            .with_remedy("Run `bp --help` to see what bp can do.");

        assert_eq!(
            envelope.to_json(),
            concat!(
                r#"{"error":"no_command","code":2,"message":"no command given","#,
                r#""problems":[],"remedy":"Run `bp --help` to see what bp can do.""#,
                "}"
            )
        );
    }

    #[test]
    fn a_typed_failure_renders_its_subject_as_a_problem() {
        let envelope = EngineError::LockTimeout {
            object: PathBuf::from("/planes/bp-a3f9c2e1/.bitplane/lock"),
        }
        .envelope();

        let json: serde_json::Value = serde_json::from_str(&envelope.to_json()).unwrap();
        assert_eq!(json["error"], "lock_timeout");
        assert_eq!(json["code"], 5);
        assert_eq!(
            json["problems"][0]["subject"],
            "/planes/bp-a3f9c2e1/.bitplane/lock"
        );
        assert!(json["remedy"].is_string());
    }

    /// Every variant, so the two tests below cover the whole enum rather than
    /// whichever variants were current when they were written.
    fn every_failure() -> Vec<EngineError> {
        vec![
            EngineError::GitMissing,
            EngineError::GitUnusable {
                message: "exited 1".to_owned(),
            },
            EngineError::GitVersionUnreadable {
                reported: "not a version at all".to_owned(),
            },
            EngineError::GitTooOld {
                found: GitVersion::new(2, 35, 9),
                required: MINIMUM_GIT_VERSION,
            },
            EngineError::InvalidRequest {
                message: "nope".to_owned(),
            },
            EngineError::LockTimeout {
                object: PathBuf::from("/planes/bp-a3f9c2e1/.bitplane/lock"),
            },
            EngineError::Io {
                path: PathBuf::from("/planes/bp-a3f9c2e1/plane.toml"),
                message: "permission denied".to_owned(),
            },
            EngineError::Refused {
                plane: "bp-a3f9c2e1".to_owned(),
                operation: Teardown::Destroy,
                blocking: 1,
                considered: 2,
                refusals: vec![a_refusal()],
            },
            EngineError::PlaneIncomplete {
                id: "bp-a3f9c2e1".to_owned(),
            },
            EngineError::PlaneNotFound {
                sought: PlaneRef::Id {
                    id: "auth-work".to_owned(),
                },
            },
            EngineError::ProjectNotInPlane {
                member: "@web".to_owned(),
                plane: "bp-a3f9c2e1".to_owned(),
            },
            EngineError::RemovalFailed {
                plane: "bp-a3f9c2e1".to_owned(),
                operation: Teardown::Remove,
                members: Vec::new(),
            },
        ]
    }

    fn a_refusal() -> Refusal {
        Refusal::new(
            crate::member::MemberRef::Repo(PathBuf::from("/repos/api")),
            crate::refusal::Reason::Uncommitted,
            "feat-login has uncommitted changes",
        )
    }

    #[test]
    fn the_envelope_tag_is_the_tag_the_wire_uses() {
        // The tag is spelled twice — in `tag()` and in serde's `tag = "error"`
        // — so a client matching on a fan-out row and one matching on stderr
        // must be matching on the same string.
        for error in every_failure() {
            let on_the_wire = serde_json::to_value(&error).unwrap();

            assert_eq!(
                on_the_wire["error"],
                error.tag(),
                "`tag()` and the serde tag disagree for {error:?}"
            );
            assert_eq!(error.envelope().error, error.tag(), "for {error:?}");
        }
    }

    #[test]
    fn every_failure_renders_a_message_and_a_code() {
        for error in every_failure() {
            let envelope = error.envelope();

            assert!(!envelope.message.is_empty(), "for {error:?}");
            assert_ne!(envelope.code, ExitCode::Ok, "for {error:?}");
            assert_ne!(envelope.code, ExitCode::Drift, "for {error:?}");
        }
    }

    #[test]
    fn a_typed_failure_round_trips_over_the_wire() {
        let error = EngineError::GitTooOld {
            found: GitVersion::new(2, 35, 9),
            required: MINIMUM_GIT_VERSION,
        };

        let json = serde_json::to_string(&error).unwrap();
        assert_eq!(
            serde_json::from_str::<EngineError>(&json).unwrap(),
            error,
            "round-tripped from {json}"
        );
    }

    #[test]
    fn a_refusal_prints_one_problem_per_reason_and_a_remedy_that_can_be_pasted() {
        let error = EngineError::Refused {
            plane: "bp-a3f9c2e1".to_owned(),
            operation: Teardown::Destroy,
            blocking: 2,
            considered: 3,
            refusals: vec![
                a_refusal(),
                Refusal::new(
                    crate::member::MemberRef::Repo(PathBuf::from("/repos/web")),
                    crate::refusal::Reason::Untracked,
                    "feat-login has untracked files",
                ),
            ],
        };

        let envelope = error.envelope();

        assert_eq!(
            envelope.message,
            "refusing to destroy bp-a3f9c2e1: 2 of 3 members have work that would be lost"
        );
        assert_eq!(envelope.problems.len(), 2);
        assert_eq!(envelope.problems[0].subject.as_deref(), Some("/repos/api"));
        assert_eq!(
            envelope.remedy.as_deref(),
            Some(
                "Inspect the members listed. Re-run with --waive uncommitted --waive untracked \
                 to accept losing that work."
            )
        );
    }

    #[test]
    fn the_two_teardown_verbs_refuse_in_their_own_words() {
        let removing = EngineError::Refused {
            plane: "bp-a3f9c2e1".to_owned(),
            operation: Teardown::Remove,
            blocking: 2,
            considered: 2,
            refusals: vec![a_refusal()],
        };

        assert_eq!(
            removing.to_string(),
            "refusing to remove 2 of 2 members from bp-a3f9c2e1: they have work that would be lost"
        );
    }

    #[test]
    fn a_plane_that_is_not_there_says_which_way_it_was_looked_for() {
        let by_id = EngineError::PlaneNotFound {
            sought: PlaneRef::Id {
                id: "auth-work".to_owned(),
            },
        };
        let from_here = EngineError::PlaneNotFound {
            sought: PlaneRef::ContainingPath {
                path: PathBuf::from("/Users/alfonz/src"),
            },
        };

        assert_eq!(by_id.to_string(), "there is no plane called auth-work");
        assert_eq!(from_here.to_string(), "no plane contains /Users/alfonz/src");
        assert!(
            by_id.envelope().remedy != from_here.envelope().remedy,
            "one way out is a listing, the other is naming a plane"
        );
    }

    #[test]
    fn a_usage_envelope_carries_the_usage_code() {
        let envelope = ErrorEnvelope::usage("no_command", "no command given")
            .with_remedy("Run `bp --help` to see what bp can do.");

        assert_eq!(envelope.code, ExitCode::Usage);
        assert!(envelope.problems.is_empty());
        assert_eq!(
            envelope.remedy.as_deref(),
            Some("Run `bp --help` to see what bp can do.")
        );
    }
}
