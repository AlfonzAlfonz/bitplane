//! The two traits every mutation and every read passes through (ADR-0003).
//!
//! The split into [`Reader`] and [`Engine`] exists so ADR-0002's *"a read never
//! writes"* is checkable by reading the trait rather than by auditing every
//! body. In Rust both reads and mutations take `&self` — the state is the
//! filesystem — so `&self`/`&mut self` cannot express the distinction.
//!
//! Sync throughout, per ADR-0001: the whole workload is spawn-subprocess-and-wait
//! plus small TOML reads and writes, so there is no IO multiplexing problem and
//! async would colour every caller for nothing.
//!
//! There is no `host` parameter. The host is chosen by picking which engine you
//! hold — [`crate::LocalEngine`] now, an SSH engine later.
//!
//! Actions arrive one slice at a time, each landing on the trait, on
//! [`crate::Request`] and in [`crate::dispatch`] together.

use crate::error::EngineError;
use crate::wire::{
    PlaneCreateRequest, PlaneCreated, PlaneList, PlaneListRequest, PlaneShowRequest, PlaneStatus,
    PlaneStatusRequest, PlaneView,
};

/// The reads: `plane_list`, `plane_show`, `plane_status`, `project_list`,
/// `project_show` and `doctor`. None of them takes a lock or creates a file.
pub trait Reader {
    /// Every plane on this host, with its members, their live branches and its
    /// health.
    ///
    /// A directory without a plane file is silently skipped; one whose plane
    /// file does not parse is a row in an error state, and the scan continues.
    /// Nothing is ever auto-repaired: a file bitplane cannot read is one it has
    /// no business rewriting.
    fn plane_list(&self, request: PlaneListRequest) -> Result<PlaneList, EngineError>;

    /// One plane, resolved by id or by the directory the caller is standing in.
    ///
    /// Unlike [`Reader::plane_list`], there is exactly one file to read, so
    /// failing to read it is a failure rather than a row.
    fn plane_show(&self, request: PlaneShowRequest) -> Result<PlaneView, EngineError>;

    /// git's own status across every member of one plane.
    ///
    /// Reading is allowed; writing is not. This stores nothing, so what it
    /// reports cannot go stale — it is git's answer, rendered (ADR-0006).
    fn plane_status(&self, request: PlaneStatusRequest) -> Result<PlaneStatus, EngineError>;
}

/// The reads, plus every mutation.
pub trait Engine: Reader {
    /// Makes a plane and a worktree of every member named.
    ///
    /// Strict, not idempotent: it either makes a whole plane or it makes none.
    /// A failure returns `Err` carrying the per-member rows, because the
    /// envelope `Err` is for an operation that produced no durable state.
    fn plane_create(&self, request: PlaneCreateRequest) -> Result<PlaneCreated, EngineError>;
}
