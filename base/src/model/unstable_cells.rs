//! The one walk that rebuilds a never-served set from stored cell state.
//!
//! Serving a stored value is only sound when that value is a function of the
//! cell's inputs. Two kinds of cell fail that test, and only one of them is
//! rebuilt here:
//!
//! - Cells on a dependency cycle, or downstream of one. A cycle has no fixed
//!   point, so what they hold is an artifact of where the walk entered. A full
//!   pass re-derives all of it every time, so incremental seeds them dirty on
//!   every pass. This set is derived from recorded edges alone -- every read
//!   that can close a cycle leaves an edge, so a stored-`#CIRC!` witness added
//!   nothing (and kept self-cycles permanently dirty); it was removed. Needing
//!   no cell state, it needs nothing from this module: the graph computes it
//!   itself in `DependencyGraph::cycle_cone`, and each scheduler installs the
//!   result after its own pass -- over the whole graph after a full one, over
//!   the cone after a selective one.
//! - Readers of a blocked spill anchor. The anchor stores `#SPILL!` but hands a
//!   same-pass reader the live array's top-left value, so only the full pass
//!   reproduces what such a reader holds. Finding them means asking what an
//!   anchor *stores*, which is exactly the cell state the graph cannot see.
//!   That is this module.
//!
//! Both sets live on [`DependencyGraph`](crate::dependency_graph::DependencyGraph),
//! which owns them and consumes them. They are also exactly the cells
//! `RecalcMode::Verify`'s stored-vs-live check has to skip.

use crate::dependency_graph::Position;
use crate::model::Model;
use crate::types::Cell;

impl Model<'_> {
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
