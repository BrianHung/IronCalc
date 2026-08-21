//! Incremental recalculation: write journal, read tracer, and schedule.
//!
//! Edges are the reads observed at evaluation time. Writes are logged by the
//! worksheet mutators and drained into dirty/force-full at `Model::evaluate`.

pub(crate) mod journal;
pub(crate) mod trace;

pub(crate) use journal::{Write, WriteLog};
pub(crate) use trace::{Input, ReadSet};
