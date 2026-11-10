//! What was written to a sheet since someone last asked.

/// One change to the cells of a sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Write {
    /// The cell at `(row, column)` was replaced, removed or handed out to be
    /// changed in place.
    Cell(i32, i32),
    /// The cells of a row were replaced or removed, all of them.
    Row(i32),
    /// Every row from `first` on moved by `delta` rows. With a negative
    /// `delta` the rows they landed on, `first + delta` to `first - 1`, are
    /// gone.
    Rows { first: i32, delta: i32 },
    /// The same for columns.
    Columns { first: i32, delta: i32 },
}

/// The writes to a sheet since the log was last taken, in the order they were
/// made, while it is recording.
///
/// Incremental recalculation reads it to find what an edit changed (see
/// `recalc`). It is not part of the sheet: it is not saved, and two sheets
/// that differ only in it are equal. It records nothing until it is told to,
/// so a model that never recalculates incrementally pays a test per write.
#[derive(Debug, Clone, Default)]
pub struct Writes {
    recording: bool,
    log: Vec<Write>,
}

impl Writes {
    pub(crate) fn push(&mut self, write: Write) {
        if self.recording && self.log.last() != Some(&write) {
            self.log.push(write);
        }
    }

    /// Starts or stops recording, and returns what was recorded so far.
    pub(crate) fn record(&mut self, recording: bool) -> Vec<Write> {
        self.recording = recording;
        std::mem::take(&mut self.log)
    }
}
