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
//! Both traits are empty today. Actions arrive one slice at a time, each landing
//! on the trait, on [`crate::Request`] and in [`crate::dispatch`] together.

/// The reads: `plane_list`, `plane_show`, `plane_status`, `project_list`,
/// `project_show` and `doctor`. None of them takes a lock or creates a file.
pub trait Reader {}

/// The reads, plus every mutation.
pub trait Engine: Reader {}
