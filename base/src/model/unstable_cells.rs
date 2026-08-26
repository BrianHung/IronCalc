//! The two sets of cells whose stored value the incremental pass may not
//! simply serve back, and the walks that rebuild them from what the last pass
//! left behind.
//!
//! Serving a stored value is only sound when that value is a function of the
//! cell's inputs. Two kinds of cell fail that test:
//!
//! - Cells on a dependency cycle, or downstream of one. A cycle has no fixed
//!   point, so what they hold is an artifact of where the walk entered. A full
//!   pass re-derives all of it every time, so incremental seeds them dirty on
//!   every pass. The cone is derived from recorded edges alone: every read
//!   that can close a cycle leaves an edge, so a stored-`#CIRC!` witness added
//!   nothing (and kept self-cycles permanently dirty); it was removed.
//! - Readers of a blocked spill anchor. The anchor stores `#SPILL!` but hands a
//!   same-pass reader the live array's top-left value, so only the full pass
//!   reproduces what such a reader holds.
//!
//! Both sets live on [`DependencyGraph`](crate::dependency_graph::DependencyGraph),
//! which owns them and consumes them; this module is only the rebuild, which
//! needs stored cell state the graph cannot see. They are also exactly the
//! cells `RecalcMode::Verify`'s stored-vs-live check has to skip.

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
