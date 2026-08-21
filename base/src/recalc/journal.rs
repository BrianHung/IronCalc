use crate::dependency_graph::Position;

/// A user-visible mutation of sheet state. Evaluation writes (storing a formula
/// result) are not journaled; they are not edits.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) enum Write {
    Value(Position),
    Formula(Position),
    Clear(Position),
    Hidden {
        sheet: u32,
        row: Option<i32>,
        column: Option<i32>,
    },
    Structural,
}

/// Per-model log of writes since the last evaluate. Worksheet mutators push;
/// `Model::evaluate` drains.
#[derive(Clone, Debug, Default)]
pub(crate) struct WriteLog {
    entries: Vec<Write>,
}

impl WriteLog {
    pub(crate) fn push(&mut self, write: Write) {
        self.entries.push(write);
    }

    #[allow(dead_code)]
    pub(crate) fn drain(&mut self) -> Vec<Write> {
        std::mem::take(&mut self.entries)
    }

    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
