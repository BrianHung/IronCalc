//! `RecalcMode::Verify`: every incremental evaluation is followed by a full
//! one, which must leave every cell as it found it.

use crate::evaluation::CellKey;
use crate::model::Model;
use crate::types::Cell;

impl Model<'_> {
    pub(crate) fn verify_against_full(&mut self) {
        let incremental = self.formula_cells();
        let links = self.links.clone();
        // Not ready: the next evaluation is full.
        self.recalc.graph = None;
        self.evaluate();
        let full = self.formula_cells();
        let differences: Vec<String> = incremental
            .iter()
            .zip(full.iter())
            .filter(|(a, b)| a != b)
            .take(10)
            .map(|(a, b)| format!("{:?}: incremental {:?}, full {:?}", a.0, a.1, b.1))
            .collect();
        assert!(
            differences.is_empty() && incremental.len() == full.len(),
            "incremental evaluation differs from a full one:\n{}",
            differences.join("\n")
        );
        assert_eq!(links, self.links, "incremental evaluation left other links");
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
                    cells.push(((sheet as u32, row, column), cell.into_owned()));
                }
            }
        }
        cells
    }
}
