//! `RecalcMode::Verify`: an evaluation that ran only some formulas is
//! followed by one that runs them all, which must leave every cell as it
//! found it and read what is on record.

use std::collections::HashSet;

use crate::evaluation::{CellKey, KeyBuild};
use crate::expressions::token::Error;
use crate::model::Model;
use crate::types::{Cell, FormulaValue};

use super::{Graph, Seeds};

impl Model<'_> {
    pub(crate) fn verify_against_everything(&mut self) {
        let Some(kept) = self.recalc.graph.take() else {
            return;
        };
        let scope = self.evaluation.scope.take();
        let restarts = self.evaluation.restarts_in_last_evaluation;
        let before = self.formula_cells();
        let links = self.links.clone();

        self.recalc.graph = Some(Graph::default());
        self.recalc.tracing = true;
        self.run();
        self.recalc.tracing = false;
        self.recalc.frames.clear();
        let after = self.formula_cells();
        let fresh = self.recalc.graph.take();

        // What reads the clock or a random number differs, and so does what
        // reads that.
        let mut kept = kept;
        let mut unsteady = kept.new_scope();
        let seeds = Seeds {
            formulas: kept.always(),
            ..Seeds::default()
        };
        kept.grow(&mut unsteady, seeds, |position| self.holds(position));
        let unsteady: HashSet<CellKey, KeyBuild> = unsteady.formulas().iter().copied().collect();
        // The marks of the scope that was evaluated are put back.
        if let Some(scope) = &scope {
            kept.restore(scope);
        }
        self.recalc.graph = Some(kept);
        // Which of two arrays that want the same cells got there first, and
        // whether a loop through the area of an array is reported circular or
        // blocked, depends on the order arrays were evaluated in before, for
        // two full evaluations as well. States that show either may differ.
        let history_dependent = before.iter().chain(after.iter()).any(|(_, cell)| {
            matches!(
                cell,
                Cell::CellFormula {
                    v: FormulaValue::Error {
                        ei: Error::CIRC | Error::SPILL,
                        ..
                    },
                    ..
                } | Cell::ArrayFormula {
                    v: FormulaValue::Error {
                        ei: Error::CIRC | Error::SPILL,
                        ..
                    },
                    ..
                }
            )
        });
        if history_dependent {
            self.evaluation.scope = scope;
            self.evaluation.restarts_in_last_evaluation = restarts;
            return;
        }
        let unsteady_formulas = unsteady;
        let unsteady = |position: &CellKey, cell: &Cell| match cell {
            Cell::SpillCell { a, .. } => unsteady_formulas.contains(&(position.0, a.0, a.1)),
            _ => unsteady_formulas.contains(position),
        };
        let any_unsteady = before
            .iter()
            .any(|(position, cell)| unsteady(position, cell));
        let after_at: std::collections::HashMap<CellKey, &Cell> = after
            .iter()
            .map(|(position, cell)| (*position, cell))
            .collect();
        let before_at: std::collections::HashMap<CellKey, &Cell> = before
            .iter()
            .map(|(position, cell)| (*position, cell))
            .collect();
        let mut differences: Vec<String> = Vec::new();
        for (position, cell) in &before {
            if unsteady(position, cell) {
                continue;
            }
            match after_at.get(position) {
                Some(full) if *full == cell => {}
                Some(full) if unsteady(position, full) => {}
                full => differences.push(format!("{position:?}: {cell:?}, in full {full:?}")),
            }
        }
        for (position, cell) in &after {
            if !before_at.contains_key(position) && !unsteady(position, cell) {
                differences.push(format!("{position:?}: nothing, in full {cell:?}"));
            }
        }
        differences.truncate(10);
        assert!(
            differences.is_empty(),
            "an incremental evaluation differs from a full one:\n{}",
            differences.join("\n")
        );
        if !any_unsteady {
            assert_eq!(
                links, self.links,
                "an incremental evaluation left other links"
            );
        }
        if let (Some(graph), Some(fresh)) = (&self.recalc.graph, &fresh) {
            if let Some(difference) = graph.difference(fresh, &unsteady_formulas) {
                panic!("the graph is not the one a full evaluation builds: {difference}");
            }
        }
        self.evaluation.scope = scope;
        self.evaluation.restarts_in_last_evaluation = restarts;
    }

    /// Every formula cell and every cell an array spilled into, in natural order.
    fn formula_cells(&self) -> Vec<(CellKey, Cell)> {
        let mut cells = Vec::new();
        for (sheet, worksheet) in self.workbook.worksheets.iter().enumerate() {
            for (row, column, cell) in worksheet.sheet_data.cells() {
                if matches!(
                    *cell,
                    Cell::CellFormula { .. } | Cell::ArrayFormula { .. } | Cell::SpillCell { .. }
                ) {
                    let mut cell = cell.into_owned();
                    // Which cell of a loop a `#CIRC!` is traced back to
                    // depends on where the evaluation entered the loop.
                    if let Cell::CellFormula { v, .. } | Cell::ArrayFormula { v, .. } = &mut cell {
                        if let FormulaValue::Error {
                            ei: Error::CIRC, d, ..
                        } = v
                        {
                            d.o.clear();
                        }
                    }
                    cells.push(((sheet as u32, row, column), cell));
                }
            }
        }
        cells
    }
}
