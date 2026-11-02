//! diskguardian-core — the scanning engine behind the DiskGuardian app.
//!
//! It answers one question honestly: *what is actually occupying this disk?*
//! Everything here is UI-free and portable: the desktop app, and any future CLI,
//! consume the same [`Tree`].
//!
//! The two invariants the whole product rests on:
//!
//! 1. Sizes are **physical allocation** (`st_blocks * 512`), never logical length.
//! 2. Space that could not be measured is **reported as restricted**, never
//!    silently dropped.

pub mod classify;
pub mod collect;
pub mod delete;
pub mod model;
pub mod scan;
pub mod view;

pub use classify::{candidates, find, AgeFilter, CandidateKind, ClassifyOptions, FindQuery, Match};
pub use collect::Collector;
pub use delete::{delete, needs_confirmation, DeleteMode, DeleteOutcome, DeleteReport, GuardError};
pub use model::{GraftError, Node, NodeId, NodeKind, Tree};
pub use scan::{
    scan, scan_with_progress, BranchDone, ProgressFn, ScanError, ScanOptions, DEADLINE_NOTE,
};
pub use view::{build_view, ViewNode, ViewOptions};
