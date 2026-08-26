//! The two records incremental recalculation is built from: what was written,
//! and what was read.
//!
//! Writes are logged by the worksheet mutators and drained into dirty/force-full
//! at `Model::evaluate`. Edges are the reads observed at evaluation time; there
//! is no static analysis of formula text. What is done with the two records --
//! the graph and the scheduler -- lives in `crate::dependency_graph` and
//! `crate::model`.

/// The write journal: what a user edit records for the next pass to consume.
pub mod journal;
/// The read tracer: what one formula observed while it evaluated.
pub(crate) mod trace;

pub(crate) use journal::Write;
pub use journal::WriteLog;
pub(crate) use trace::{Input, ReadSet};
