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

pub mod create;
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
pub mod plane_dir;
pub mod plane_file;
pub mod plane_id;
pub mod read;
pub mod repo;
pub mod termination;
pub mod time;
pub mod wire;

#[doc(hidden)]
pub mod testing;

pub use directories::{Directories, DirectoryOverrides, Environment, SystemEnvironment};
pub use engine::{Engine, Reader};
pub use error::{BranchWanted, EngineError, ErrorEnvelope, Occupant, Problem};
pub use exit::ExitCode;
pub use git::{
    GitPrerequisite, GitProbe, GitVersion, MINIMUM_GIT_VERSION, ProbeFailure, SystemGit,
};
pub use head::Head;
pub use health::{Finding, HealthCheck, PlaneHealth};
pub use interrupt::Interrupt;
pub use local::LocalEngine;
pub use member::{MemberRef, ProjectName, WorktreePath};
pub use outcome::{Outcome, PerMember, SkipReason};
pub use plane_file::{Member, PlaneFile};
pub use plane_id::PlaneId;
pub use termination::Termination;
pub use wire::{
    BranchIntent, CreatedMember, MemberStatus, MemberView, MemberWork, PlaneCreateRequest,
    PlaneCreated, PlaneList, PlaneListRequest, PlaneRef, PlaneShowRequest, PlaneStatus,
    PlaneStatusRequest, PlaneView, Request, Response, dispatch,
};
