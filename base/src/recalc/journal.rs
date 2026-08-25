use std::ops::{Deref, DerefMut};

use crate::dependency_graph::Position;
use crate::model::Model;

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
    /// A hyperlink was attached to or removed from a cell. The link is part of
    /// the cell's observable key, so its readers and any delta must see it.
    Link { at: Position },
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

    /// Private to this module on purpose: [`JournalRecordingPaused`] is the
    /// only way to stop recording, and it always turns it back on.
    fn set_recording(&mut self, recording: bool) {
        self.recording = recording;
    }
}

/// Scoped pause of the write journal: the one legal bypass of "every user
/// edit is journaled".
///
/// Two callers need it. [`Model::evaluate`] pauses while it recomputes,
/// because storing a formula result is not an edit. `write_displaced_formula`
/// pauses around the raw write and then pushes its own entry, because a
/// displacement must be journaled as a value write rather than a formula one.
///
/// Both used to be a `set_recording(false)` ... `set_recording(true)` pair,
/// which an early `?` between the two would leak: from then on the workbook
/// would silently stop journaling and the incremental pass would miss every
/// later edit. The guard makes that unrepresentable. It *is* the mutable
/// handle to the model — the paused work has to run through it — and `Drop`
/// restores the previous state on every exit path, including `?`, `return`
/// and unwinding.
///
/// The previous state is saved rather than assumed, so pauses may nest.
#[must_use = "the journal is paused only while this guard is alive"]
pub(crate) struct JournalRecordingPaused<'a, 'm> {
    model: &'a mut Model<'m>,
    /// `(worksheet index, recording state to restore)`.
    restore: Vec<(usize, bool)>,
}

impl<'a, 'm> JournalRecordingPaused<'a, 'm> {
    fn pause(model: &'a mut Model<'m>, sheets: Option<usize>) -> Self {
        let mut restore = Vec::new();
        let worksheets = &mut model.workbook.worksheets;
        match sheets {
            Some(index) => {
                if let Some(worksheet) = worksheets.get_mut(index) {
                    restore.push((index, worksheet.write_log.recording));
                    worksheet.write_log.set_recording(false);
                }
            }
            None => {
                for (index, worksheet) in worksheets.iter_mut().enumerate() {
                    restore.push((index, worksheet.write_log.recording));
                    worksheet.write_log.set_recording(false);
                }
            }
        }
        Self { model, restore }
    }
}

impl<'m> Deref for JournalRecordingPaused<'_, 'm> {
    type Target = Model<'m>;

    fn deref(&self) -> &Self::Target {
        self.model
    }
}

impl DerefMut for JournalRecordingPaused<'_, '_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.model
    }
}

impl Drop for JournalRecordingPaused<'_, '_> {
    fn drop(&mut self) {
        for &(index, recording) in &self.restore {
            if let Some(worksheet) = self.model.workbook.worksheets.get_mut(index) {
                worksheet.write_log.set_recording(recording);
            }
        }
    }
}

impl<'m> Model<'m> {
    /// Pauses the journal on every sheet until the guard is dropped.
    pub(crate) fn pause_journal(&mut self) -> JournalRecordingPaused<'_, 'm> {
        JournalRecordingPaused::pause(self, None)
    }

    /// Pauses the journal on one sheet until the guard is dropped. An
    /// out-of-range `sheet` pauses nothing, as the raw pair did.
    pub(crate) fn pause_journal_for_sheet(&mut self, sheet: u32) -> JournalRecordingPaused<'_, 'm> {
        JournalRecordingPaused::pause(self, Some(sheet as usize))
    }
}
