//! The bitplane machine contract.
//!
//! "Machine contract first; the human CLI renders it" — this crate owns the
//! [`Engine`] traits, the wire types, the exit codes and the error envelope;
//! `bitplane-cli` renders them (ADR-0001). An MCP surface or an SSH servant
//! would be further renderings of the same types, which is why the envelope is
//! constructed here and not at each surface.
//!
//! Paths are held as path types throughout and never as strings. ADR-0001 defers
//! Windows, and that deferral binds exactly this one constraint.

pub mod add;
pub mod create;
pub mod destroy;
pub mod directories;
pub mod engine;
pub mod error;
pub mod exit;
pub mod fsio;
pub mod git;
pub mod head;
pub mod health;
pub mod interrupt;
pub mod local;
pub mod lock;
pub mod member;
pub mod outcome;
pub mod plan;
pub mod plane_dir;
pub mod plane_file;
pub mod plane_id;
pub mod project_add;
pub mod project_dir;
pub mod project_fetch;
pub mod project_file;
pub mod project_list;
pub mod read;
pub mod refusal;
pub mod repo;
pub mod run;
pub mod scripts;
pub mod termination;
pub mod time;
pub mod toml_text;
pub mod wire;
pub mod worktrees;

#[doc(hidden)]
pub mod testing;

pub use directories::{Directories, DirectoryOverrides, Environment, SystemEnvironment};
pub use engine::{Engine, Reader};
pub use error::{BranchWanted, EngineError, ErrorEnvelope, Occupant, Problem, Teardown};
pub use exit::ExitCode;
pub use git::{
    GitPrerequisite, GitProbe, GitVersion, MINIMUM_GIT_VERSION, ProbeFailure, SystemGit,
};
pub use head::Head;
pub use health::{Finding, HealthCheck, PlaneHealth};
pub use interrupt::Interrupt;
pub use local::LocalEngine;
pub use member::{MemberRef, ProjectName, WorktreePath};
pub use outcome::{Outcome, PerMember, PerProject, SkipReason};
pub use plane_file::{Member, PlaneFile};
pub use plane_id::PlaneId;
pub use project_file::{ProjectFile, ProjectSource};
pub use refusal::{Reason, Refusal, Waivers};
pub use scripts::{Script, ScriptCommand, ScriptName, ScriptPoint};
pub use termination::Termination;
pub use wire::{
    BranchDisposition, BranchIntent, CreatedMember, Fetched, MemberStatus, MemberView, MemberWork,
    PlaneAddRequest, PlaneAdded, PlaneCreateRequest, PlaneCreated, PlaneDestroyRequest,
    PlaneDestroyed, PlaneList, PlaneListRequest, PlaneRef, PlaneRemoveRequest, PlaneRemoved,
    PlaneScriptsRequest, PlaneShowRequest, PlaneStatus, PlaneStatusRequest, PlaneView,
    ProjectAddRequest, ProjectAdded, ProjectFetchRequest, ProjectFetched, ProjectListing,
    ProjectSummary, RemovedMember, Request, Response, ScriptOutcome, ScriptResult, ScriptsRun,
    dispatch,
};
