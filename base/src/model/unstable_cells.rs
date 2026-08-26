//! The two sets of cells whose stored value the incremental pass may not
//! simply serve back, and the walks that rebuild them from what the last pass
//! left behind.
//!
//! Serving a stored value is only sound when that value is a function of the
//! cell's inputs. Two kinds of cell fail that test:
//!
//! - Cells on a dependency cycle, downstream of one, or reporting `#CIRC!`. A
//!   cycle has no fixed point, so what they hold is an artifact of where the
//!   walk entered. A full pass re-derives all of it every time, so incremental
//!   seeds them dirty on every pass.
//! - Readers of a blocked spill anchor. The anchor stores `#SPILL!` but hands a
//!   same-pass reader the live array's top-left value, so only the full pass
//!   reproduces what such a reader holds.
//!
//! Both sets live on [`DependencyGraph`](crate::dependency_graph::DependencyGraph),
//! which owns them and consumes them; this module is only the rebuild, which
//! needs stored cell state the graph cannot see. They are also exactly the
//! cells `RecalcMode::Verify`'s stored-vs-live check has to skip.

use std::collections::HashSet;

use crate::dependency_graph::Position;
use crate::model::Model;
use crate::types::Cell;

impl Model<'_> {
    /// Rebuilds the set of cells whose last result was not a genuine function
    /// value: `cone`, the cells the graph could not order, plus every cell in
    /// `scope` that reported `#CIRC!`.
    ///
    /// The witness is not redundant with the cone. The cone is what the
    /// recorded edges say; a stored `#CIRC!` is the evaluator's own report that
    /// a cell re-entered itself, and it stands even if the read that closed the
    /// loop left no edge behind.
    pub(crate) fn refresh_unstable_cells(
        &mut self,
        mut cone: HashSet<Position>,
        scope: &HashSet<Position>,
    ) {
        cone.extend(
            scope
                .iter()
                .copied()
                .filter(|&position| self.stores_circular_error(position)),
        );
        self.graph.set_never_served(cone);
    }

    /// Whether the cell at `position` reported `#CIRC!`: the evaluator's own
    /// record that it was re-entered while it was still evaluating.
    fn stores_circular_error(&self, position: Position) -> bool {
        use crate::expressions::token::Error::CIRC;
        use crate::types::{FormulaValue, SpillValue};
        match self.cell_at(position) {
            Some(
                Cell::CellFormula {
                    v: FormulaValue::Error { ei, .. },
                    ..
                }
                | Cell::ArrayFormula {
                    v: FormulaValue::Error { ei, .. },
                    ..
                },
            ) => *ei == CIRC,
            Some(Cell::SpillCell {
                v: SpillValue::Error(ei),
                ..
            }) => *ei == CIRC,
            _ => false,
        }
    }

    /// Rebuilds the readers of blocked spill anchors: the cells a full pass
    /// hands the live array's top-left value while the anchor itself stores
    /// `#SPILL!`. Run only from the full pass, which is the only pass that can
    /// block or unblock an anchor -- an evaluation write to an array footprint
    /// sends the pass to full -- and the only pass whose walk sees every
    /// anchor. A stale entry costs one conservative full fallback.
    pub(crate) fn refresh_blocked_array_readers(&mut self) {
        use crate::expressions::token::Error::SPILL;
        use crate::types::FormulaValue;
        let blocked: Vec<Position> = self
            .graph
            .arrays
            .snapshot()
            .into_iter()
            .filter(|&position| {
                matches!(
                    self.cell_at(position),
                    Some(Cell::ArrayFormula {
                        v: FormulaValue::Error { ei: SPILL, .. },
                        ..
                    })
                )
            })
            .collect();
        let readers = blocked
            .into_iter()
            .flat_map(|anchor| self.graph.dependents_of(anchor))
            .collect();
        self.graph.set_blocked_array_readers(readers);
    }
}
