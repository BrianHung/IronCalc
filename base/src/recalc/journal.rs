use crate::dependency_graph::Position;

/// A user-visible mutation of sheet state. Evaluation writes (storing a formula
/// result) are not journaled; they are not edits.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Write {
    /// A cell's content changed. `was_formula` lets the consumer drop stale
    /// outgoing edges.
    Cell {
        at: Position,
        was_formula: bool,
        is_formula: bool,
    },
    Hidden {
        sheet: u32,
        row: Option<i32>,
        column: Option<i32>,
    },
}

/// Per-worksheet log of writes since the last evaluate. Worksheet mutators
/// push; `Model::evaluate` drains.
#[derive(Clone, Debug, PartialEq)]
pub struct WriteLog {
    entries: Vec<Write>,
    recording: bool,
}

impl Default for WriteLog {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            recording: true,
        }
    }
}

impl WriteLog {
    pub(crate) fn push(&mut self, write: Write) {
        if self.recording {
            self.entries.push(write);
        }
    }

    pub(crate) fn drain(&mut self) -> Vec<Write> {
        std::mem::take(&mut self.entries)
    }

    pub(crate) fn set_recording(&mut self, recording: bool) {
        self.recording = recording;
    }

    pub(crate) fn is_recording(&self) -> bool {
        self.recording
    }
}
