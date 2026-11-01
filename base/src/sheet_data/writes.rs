//! What was written to a sheet since someone last asked.

/// The writes to a sheet since the log was last taken, while it is recording.
///
/// Incremental recalculation reads it to find what an edit changed (see
/// `recalc`). It is not part of the sheet: it is not saved, and two sheets
/// that differ only in it are equal. It records nothing until it is told to,
/// so a model that never recalculates incrementally pays a test per write.
#[derive(Debug, Clone, Default)]
pub struct Writes {
    recording: bool,
    /// Positions `(row, column)` whose cell was replaced, removed or handed
    /// out to be changed in place. A position can appear more than once.
    pub(crate) cells: Vec<(i32, i32)>,
    /// Whether rows or columns were moved, removed or replaced whole, which
    /// moves cells rather than writing them.
    pub(crate) structural: bool,
}

impl Writes {
    pub(crate) fn cell(&mut self, row: i32, column: i32) {
        if self.recording {
            self.cells.push((row, column));
        }
    }

    pub(crate) fn structural(&mut self) {
        if self.recording {
            self.structural = true;
        }
    }

    /// Starts or stops recording. Either way what was recorded is dropped.
    pub(crate) fn record(&mut self, recording: bool) {
        self.recording = recording;
        self.cells.clear();
        self.structural = false;
    }
}
