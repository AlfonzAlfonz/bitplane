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

pub mod engine;
pub mod error;
pub mod exit;
pub mod git;
pub mod local;
pub mod termination;
pub mod wire;

pub use engine::{Engine, Reader};
pub use error::{EngineError, ErrorEnvelope, Problem};
pub use exit::ExitCode;
pub use git::{
    GitPrerequisite, GitProbe, GitVersion, MINIMUM_GIT_VERSION, ProbeFailure, SystemGit,
};
pub use local::LocalEngine;
pub use termination::Termination;
pub use wire::{Request, Response, dispatch};
