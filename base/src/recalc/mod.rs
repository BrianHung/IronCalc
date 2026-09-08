//! The write journal: what a user edit records for a later consumer.
//!
//! Writes are logged by the worksheet mutators and drained at
//! `Model::evaluate`. Storing a formula's result is not a write, so the
//! journal is paused for the whole of a pass. What is *done* with the record
//! lives elsewhere: today, `crate::changed_cells` turns it into the delta the
//! `take_changed_cells` API reports.

/// The write journal: what a user edit records for the next pass to consume.
pub mod journal;

pub(crate) use journal::Write;
pub use journal::WriteLog;

/// A cell address that carries its sheet: `(sheet index, row, column)`.
///
/// Lives here because the journal is its first user. It is deliberately the
/// same shape as the keys of `Model::links` and `Model::cf_cache`, which the
/// change-key diff snapshots against it.
pub(crate) type Position = (u32, i32, i32);
