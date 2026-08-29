use crate::cf_types::{CfRule, Cfvo};
use crate::constants::{LAST_COLUMN, LAST_ROW};
use crate::cut_paste::cf_sqref_anchor;
use crate::dependency_graph::Axis;
use crate::expressions::parser::displace::displace_node;
use crate::expressions::parser::static_analysis::{run_static_analysis_on_node, StaticResult};
use crate::expressions::parser::stringify::{
    to_localized_string, to_rc_format, to_string_displaced, DisplaceData,
};
use crate::expressions::parser::Node;
use crate::expressions::parser::Parser as ExprParser;
use crate::expressions::types::CellReferenceRC;
use crate::expressions::utils;
use crate::language::get_default_language;
use crate::locale::get_default_locale;
use crate::model::{CellStructure, Model};
use crate::types::{ArrayKind, Cell, Link, Worksheet};

/// A cell lifted out of a moved row or column, ready to be written back:
/// target row and column, formula or value, style index, and the CSE
/// rectangle it anchors, if any.
type MovedCell = (i32, i32, String, i32, Option<(i32, i32)>);

/// Applies `map` to the (row, column) key of every link in the worksheet, so
/// that links follow their cells when rows or columns are inserted, deleted or
/// moved: `Some((row, column))` moves the link there, `None` removes it.
/// Every moved, added or removed position is journaled: a link is part of the
/// cell's observable state and its readers must re-run.
fn displace_links<F>(worksheet: &mut Worksheet, sheet: u32, map: F)
where
    F: Fn(i32, i32) -> Option<(i32, i32)>,
{
    let links = std::mem::take(&mut worksheet.links);
    worksheet.links = links
        .into_iter()
        .filter_map(|((row, column), link)| {
            let new_key = map(row, column);
            if new_key != Some((row, column)) {
                worksheet.write_log.push(crate::recalc::Write::Link {
                    at: (sheet, row, column),
                });
            }
            new_key.map(|key| (key, link))
        })
        .collect();
}

/// Returns the new row after displacement, or `None` if the row was deleted.
fn displace_cf_row(row: i32, data: &DisplaceData, sheet: u32) -> Option<i32> {
    match data {
        DisplaceData::Row {
            sheet: s,
            row: dr,
            delta,
        } if *s == sheet => {
            if row >= *dr {
                if *delta < 0 && row < *dr - *delta {
                    None
                } else {
                    Some(row + *delta)
                }
            } else {
                Some(row)
            }
        }
        DisplaceData::RowMove {
            sheet: s,
            row: mr,
            delta,
        } if *s == sheet => {
            if row == *mr {
                Some(row + *delta)
            } else if *delta > 0 && row > *mr && row <= *mr + *delta {
                Some(row - 1)
            } else if *delta < 0 && row < *mr && row >= *mr + *delta {
                Some(row + 1)
            } else {
                Some(row)
            }
        }
        _ => Some(row),
    }
}

/// Returns the new column after displacement, or `None` if the column was deleted.
fn displace_cf_col(col: i32, data: &DisplaceData, sheet: u32) -> Option<i32> {
    match data {
        DisplaceData::Column {
            sheet: s,
            column: dc,
            delta,
        } if *s == sheet => {
            if col >= *dc {
                if *delta < 0 && col < *dc - *delta {
                    None
                } else {
                    Some(col + *delta)
                }
            } else {
                Some(col)
            }
        }
        DisplaceData::ColumnMove {
            sheet: s,
            column: mc,
            delta,
        } if *s == sheet => {
            if col == *mc {
                Some(col + *delta)
            } else if *delta > 0 && col > *mc && col <= *mc + *delta {
                Some(col - 1)
            } else if *delta < 0 && col < *mc && col >= *mc + *delta {
                Some(col + 1)
            } else {
                Some(col)
            }
        }
        _ => Some(col),
    }
}

/// Displaces a single A1-style sqref part (e.g. "A1" or "A1:B5").
/// Returns the original string unchanged if any corner would become #REF!.
fn displace_cf_sqref_part(part: &str, data: &DisplaceData, sheet: u32) -> String {
    let upper = part.to_uppercase();
    let segs: Vec<&str> = upper.splitn(2, ':').collect();
    match segs.len() {
        1 => {
            if let Some(r) = utils::parse_reference_a1(segs[0]) {
                if let (Some(nr), Some(nc)) = (
                    displace_cf_row(r.row, data, sheet),
                    displace_cf_col(r.column, data, sheet),
                ) {
                    if let Some(c) = utils::number_to_column(nc) {
                        return format!("{c}{nr}");
                    }
                }
            }
            part.to_string()
        }
        2 => {
            if let (Some(r1), Some(r2)) = (
                utils::parse_reference_a1(segs[0]),
                utils::parse_reference_a1(segs[1]),
            ) {
                if let (Some(nr1), Some(nc1), Some(nr2), Some(nc2)) = (
                    displace_cf_row(r1.row, data, sheet),
                    displace_cf_col(r1.column, data, sheet),
                    displace_cf_row(r2.row, data, sheet),
                    displace_cf_col(r2.column, data, sheet),
                ) {
                    if let (Some(c1), Some(c2)) =
                        (utils::number_to_column(nc1), utils::number_to_column(nc2))
                    {
                        return format!("{c1}{nr1}:{c2}{nr2}");
                    }
                }
            }
            part.to_string()
        }
        _ => part.to_string(),
    }
}

/// Displaces every part of a space-separated sqref string.
fn displace_cf_sqref(sqref: &str, data: &DisplaceData, sheet: u32) -> String {
    sqref
        .split_whitespace()
        .map(|p| displace_cf_sqref_part(p, data, sheet))
        .collect::<Vec<_>>()
        .join(" ")
}

// NOTE: There is a difference with Excel behaviour when deleting cells/rows/columns
// In Excel if the whole range is deleted then it will substitute for #REF!
// In IronCalc, if one of the edges of the range is deleted will replace the edge with #REF!
// I feel this is unimportant for now.

/// Displaces a single formula string (with or without leading `=`) using `to_string_displaced`.
/// CF formulas are stored in English (see [Model::user_formula_to_internal]),
/// so the caller must have the parser in the default (English) locale/language.
fn displace_cf_formula_str(
    parser: &mut ExprParser<'_>,
    formula: &str,
    context: &CellReferenceRC,
    data: &DisplaceData,
) -> String {
    let trimmed = formula.trim();
    let has_eq = trimmed.starts_with('=');
    let body = if has_eq { &trimmed[1..] } else { trimmed };
    let node = parser.parse(body, context);
    let displaced = to_string_displaced(
        &node,
        context,
        data,
        get_default_locale(),
        get_default_language(),
    );
    if has_eq {
        format!("={displaced}")
    } else {
        displaced
    }
}

fn displace_cfvo(
    parser: &mut ExprParser<'_>,
    cfvo: Cfvo,
    context: &CellReferenceRC,
    data: &DisplaceData,
) -> Cfvo {
    if let Cfvo::Formula(f) = cfvo {
        Cfvo::Formula(displace_cf_formula_str(parser, &f, context, data))
    } else {
        cfvo
    }
}

/// Displaces all formula fields inside a `CfRule`.
fn displace_cf_rule_formulas(
    parser: &mut ExprParser<'_>,
    rule: CfRule,
    context: &CellReferenceRC,
    data: &DisplaceData,
) -> CfRule {
    match rule {
        CfRule::Formula {
            formula,
            dxf_id,
            stop_if_true,
        } => CfRule::Formula {
            formula: displace_cf_formula_str(parser, &formula, context, data),
            dxf_id,
            stop_if_true,
        },
        CfRule::CellIs {
            operator,
            formula,
            formula2,
            dxf_id,
            stop_if_true,
        } => CfRule::CellIs {
            operator,
            formula: displace_cf_formula_str(parser, &formula, context, data),
            formula2: formula2.map(|f| displace_cf_formula_str(parser, &f, context, data)),
            dxf_id,
            stop_if_true,
        },
        CfRule::ColorScale { thresholds } => CfRule::ColorScale {
            thresholds: thresholds
                .into_iter()
                .map(|mut t| {
                    t.cfvo = displace_cfvo(parser, t.cfvo, context, data);
                    t
                })
                .collect(),
        },
        CfRule::DataBar {
            min,
            max,
            positive_color,
            negative_color,
            is_gradient,
            show_value,
        } => CfRule::DataBar {
            min: min.map(|c| displace_cfvo(parser, c, context, data)),
            max: max.map(|c| displace_cfvo(parser, c, context, data)),
            positive_color,
            negative_color,
            is_gradient,
            show_value,
        },
        CfRule::IconSet {
            thresholds,
            show_value,
        } => CfRule::IconSet {
            thresholds: thresholds
                .into_iter()
                .map(|mut t| {
                    t.cfvo = displace_cfvo(parser, t.cfvo, context, data);
                    t
                })
                .collect(),
            show_value,
        },
        CfRule::IconRating {
            icon,
            color,
            thresholds,
            show_value,
        } => CfRule::IconRating {
            icon,
            color,
            thresholds: thresholds
                .into_iter()
                .map(|(cfvo, strict)| (displace_cfvo(parser, cfvo, context, data), strict))
                .collect(),
            show_value,
        },
        // No formula fields in remaining variants
        other => other,
    }
}

impl<'a> Model<'a> {
    fn shift_cell_formula(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        displace_data: &DisplaceData,
    ) -> Result<(), String> {
        let (formula_index, is_plain_formula) = {
            let Some(cell) = self.workbook.worksheet(sheet)?.cell(row, column) else {
                return Ok(());
            };
            match cell.get_formula() {
                Some(f) => (f, matches!(cell, Cell::CellFormula { .. })),
                None => return Ok(()),
            }
        };
        let node = self.parsed_formulas[sheet as usize][formula_index as usize]
            .0
            .clone();

        // Fast path: rewrite the AST directly, avoiding the stringify + reparse
        // round trip. Only plain formula cells are handled; a reference displaced
        // off the sheet (#REF!), a malformed reference, or an array/dynamic cell
        // returns from `displace_node` as `None` and falls back to the exact
        // string path below.
        if is_plain_formula {
            if let Some(new_node) = displace_node(&node, row, column, displace_data) {
                let new_rc = to_rc_format(&new_node);
                if new_rc == self.workbook.worksheet(sheet)?.shared_formulas[formula_index as usize]
                {
                    return Ok(());
                }
                // Imported plain formulas can static-analyze non-Scalar (defined
                // names, A1:expr, spill). The string path upgrades those to
                // dynamic; asserting Scalar panics in debug and diverges in release.
                if matches!(run_static_analysis_on_node(&new_node), StaticResult::Scalar) {
                    return self.set_displaced_formula(sheet, row, column, new_node, new_rc);
                }
            }
        }

        // Fallback: render the displaced formula and write it back through the
        // parser. Both strings must be in the active locale/language: the
        // displaced one is written back through the (localized) parser, and
        // comparing against an English rendering would flag every formula.
        let cell_reference = CellReferenceRC {
            sheet: self.workbook.worksheets[sheet as usize].get_name(),
            row,
            column,
        };
        let formula = to_localized_string(&node, &cell_reference, self.locale, self.language);
        let formula_displaced = to_string_displaced(
            &node,
            &cell_reference,
            displace_data,
            self.locale,
            self.language,
        );
        if formula != formula_displaced {
            self.write_displaced_formula(sheet, row, column, format!("={formula_displaced}"))?;
        }
        Ok(())
    }

    /// Stores an already-displaced formula AST for a plain formula cell, reusing
    /// a matching shared formula or appending a new one.
    fn set_displaced_formula(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        node: Node,
        rc: String,
    ) -> Result<(), String> {
        let static_result = run_static_analysis_on_node(&node);
        let mut style = self.workbook.worksheet(sheet)?.get_style(row, column);
        if self.workbook.styles.style_is_quote_prefix(style) {
            style = self.workbook.styles.get_style_without_quote_prefix(style)?;
        }
        let existing = self
            .workbook
            .worksheet(sheet)?
            .shared_formulas
            .iter()
            .position(|f| f == &rc);
        let index = if let Some(i) = existing {
            i as i32
        } else {
            let worksheet = self.workbook.worksheet_mut(sheet)?;
            worksheet.shared_formulas.push(rc);
            (worksheet.shared_formulas.len() as i32) - 1
        };
        if existing.is_none() {
            self.parsed_formulas[sheet as usize].push((node, static_result));
        }
        self.workbook
            .worksheet_mut(sheet)?
            .set_cell_with_formula(row, column, index, style)?;
        Ok(())
    }
    /// This function iterates over all cells in the model and shifts their formulas according to the displacement data.
    ///
    /// # Arguments
    ///
    /// * `displace_data` - A reference to `DisplaceData` describing the displacement's direction and magnitude.
    fn displace_cells(&mut self, displace_data: &DisplaceData) -> Result<(), String> {
        let cells = self.get_all_cells();
        for cell in cells {
            self.shift_cell_formula(cell.index, cell.row, cell.column, displace_data)?;
        }
        Ok(())
    }

    /// Updates the `range` field and formula fields of every CF rule on `sheet` according to `displace_data`.
    fn displace_cf_ranges(&mut self, sheet: u32, displace_data: &DisplaceData) {
        let count = match self.workbook.worksheets.get(sheet as usize) {
            Some(ws) => ws.conditional_formatting.len(),
            None => return,
        };

        // Phase 1: collect (index, new_range, old_rule, anchor) without holding a borrow on self.
        let sheet_name = self.workbook.worksheets[sheet as usize].get_name();
        let mut phase1: Vec<(usize, String, CfRule, i32, i32)> = Vec::with_capacity(count);
        for idx in 0..count {
            let cf = &self.workbook.worksheets[sheet as usize].conditional_formatting[idx];
            let old_range = cf.range.clone();
            let new_range = displace_cf_sqref(&old_range, displace_data, sheet);
            let rule = cf.cf_rule.clone();
            if let Some((anchor_row, anchor_col)) = cf_sqref_anchor(&old_range) {
                phase1.push((idx, new_range, rule, anchor_row, anchor_col));
            }
        }

        // Phase 2: displace formula fields (requires &mut self.parser) then write back.
        // CF formulas are stored in English, so parse them with the default
        // locale/language regardless of the active ones.
        let locale = self.locale;
        let language = self.language;
        self.parser.set_locale(get_default_locale());
        self.parser.set_language(get_default_language());
        for (idx, new_range, rule, anchor_row, anchor_col) in phase1 {
            let context = CellReferenceRC {
                sheet: sheet_name.clone(),
                row: anchor_row,
                column: anchor_col,
            };
            let new_rule =
                displace_cf_rule_formulas(&mut self.parser, rule, &context, displace_data);
            self.workbook.worksheets[sheet as usize].conditional_formatting[idx].range = new_range;
            self.workbook.worksheets[sheet as usize].conditional_formatting[idx].cf_rule = new_rule;
        }
        self.parser.set_locale(locale);
        self.parser.set_language(language);
    }

    /// Retrieves the column indices for a specific row in a given sheet, sorted in ascending or descending order.
    ///
    /// # Arguments
    ///
    /// * `sheet` - The sheet number to retrieve columns from.
    /// * `row` - The row number to retrieve columns for.
    /// * `descending` - If true, the columns are returned in descending order; otherwise, in ascending order.
    ///
    /// # Returns
    ///
    /// This function returns a `Result` containing either:
    /// - `Ok(Vec<i32>)`: A vector of column indices for the specified row, sorted according to the `descending` flag.
    /// - `Err(String)`: An error message if the sheet cannot be found.
    fn get_columns_for_row(
        &self,
        sheet: u32,
        row: i32,
        descending: bool,
    ) -> Result<Vec<i32>, String> {
        let worksheet = self.workbook.worksheet(sheet)?;
        if let Some(row_data) = worksheet.sheet_data.get(&row) {
            let mut columns: Vec<i32> = row_data.keys().copied().collect();
            columns.sort_unstable();
            if descending {
                columns.reverse();
            }
            Ok(columns)
        } else {
            Ok(vec![])
        }
    }

    /// The write half of [`Model::move_cell`].
    ///
    /// An array formula rewrites its whole range. A plain formula is written
    /// graph-neutrally, rather than through `set_user_input` which forces a
    /// full recompute, so the structural edit's edge shift can keep the next
    /// pass incremental. A value goes through the normal input path.
    fn move_cell_write(
        &mut self,
        sheet: u32,
        target_row: i32,
        target_column: i32,
        array: Option<(i32, i32)>,
        formula_or_value: &str,
    ) -> Result<(), String> {
        if let Some((width, height)) = array {
            self.set_user_array_formula(
                sheet,
                target_row,
                target_column,
                width,
                height,
                formula_or_value,
            )?;
        } else if let Some(formula) = formula_or_value.strip_prefix('=') {
            self.write_displaced_formula(sheet, target_row, target_column, format!("={formula}"))?;
        } else {
            self.set_user_input(
                sheet,
                target_row,
                target_column,
                formula_or_value.to_string(),
            )?;
        }
        Ok(())
    }

    /// Writes the cells lifted out of a moved row or column back at their new
    /// positions. Callers wrap it in [`Model::with_cse_guard_suspended`]: the
    /// rebuild writes the anchor of a CSE array and, right after it, the
    /// placeholders of the rectangle the anchor has just re-declared.
    ///
    /// Deliberately NOT unified with [`Model::move_cell_write`], which it
    /// resembles. That path writes a plain formula through
    /// `write_displaced_formula`, which journals `is_formula: false` because
    /// what it performs is a *rewrite* of text a displacement changed. This
    /// rebuild re-creates a whole line's cells at positions it lifted them
    /// away from, so `set_user_input` is the honest entry: the journal sees a
    /// formula appearing where one may not have been, and the spill and
    /// quote-prefix preparation that a fresh write needs runs. Each lifted
    /// cell's style is restored inline too, which `move_cell` leaves to its
    /// caller.
    fn rebuild_moved_cells(&mut self, sheet: u32, cells: Vec<MovedCell>) -> Result<(), String> {
        for (row, column, value, style_index, array) in cells {
            if let Some((width, height)) = array {
                self.set_user_array_formula(sheet, row, column, width, height, &value)?;
            } else {
                self.set_user_input(sheet, row, column, value)?;
            }
            self.workbook
                .worksheet_mut(sheet)?
                .set_cell_style(row, column, style_index)?;
        }
        Ok(())
    }

    /// Moves the contents of cell (source_row, source_column) to (target_row, target_column).
    ///
    /// It assumes that the caller has already checked that the move is valid
    /// (e.g. it does not split an array formula). And that dynamic array spills have been reset.
    fn move_cell(
        &mut self,
        sheet: u32,
        source_row: i32,
        source_column: i32,
        target_row: i32,
        target_column: i32,
    ) -> Result<(), String> {
        let source_cell = match self
            .workbook
            .worksheet(sheet)?
            .cell(source_row, source_column)
        {
            Some(c) => c,
            None => return Ok(()),
        };
        let style = source_cell.get_style();

        let mut array = None;

        match source_cell {
            Cell::EmptyCell { .. }
            | Cell::BooleanCell { .. }
            | Cell::NumberCell { .. }
            | Cell::ErrorCell { .. }
            | Cell::SharedString { .. }
            | Cell::CellFormula { .. } => {
                // This is a regular cell, we can just move it.
            }
            Cell::SpillCell { .. } => {
                // This the spill of an array formula. Because dynamic arrays spills have been deleted
                // We delete the spill
                let worksheet = self.workbook.worksheet_mut(sheet)?;
                // Sanctioned: vacating the source of a cell move. The position is meant
                // to lose its style too, unlike a footprint teardown.
                #[allow(clippy::disallowed_methods)]
                worksheet.remove_cell(source_row, source_column)?;
                return Ok(());
            }
            Cell::ArrayFormula {
                r,
                kind: ArrayKind::Dynamic,
                ..
            } => {
                // We are moving the anchor of a dynamic formula.
                // We assume the spill has been taken care of by the caller
                debug_assert_eq!(*r, (1, 1));
            }
            Cell::ArrayFormula {
                r,
                kind: ArrayKind::Cse,
                ..
            } => {
                // This is an array formula, we need to move the whole range
                // We rely on the calling function to check that the move is valid and does not split the array formula
                array = Some(*r);
            }
        }
        let formula_or_value = self
            .get_cell_formula(sheet, source_row, source_column)?
            .unwrap_or_else(|| {
                source_cell.get_localized_text(
                    &self.workbook.shared_strings,
                    self.locale,
                    self.language,
                )
            });

        self.with_cse_guard_suspended(|model| {
            model.move_cell_write(sheet, target_row, target_column, array, &formula_or_value)
        })?;

        let worksheet = self.workbook.worksheet_mut(sheet)?;
        // copy style
        worksheet.set_cell_style(target_row, target_column, style)?;

        // delete source cell content and style
        // Sanctioned: vacating the source of a cell move. The position is meant
        // to lose its style too, unlike a footprint teardown.
        #[allow(clippy::disallowed_methods)]
        worksheet.remove_cell(source_row, source_column)?;
        Ok(())
    }

    /// Replaces every link sitting on the moved line with the ones lifted off
    /// the source line, discarding whatever the cell rebuild auto-created.
    ///
    /// `moved_links` is keyed by the coordinate that is *not* the line: the
    /// column for a row move, the row for a column move.
    ///
    /// Both the discarded positions and the re-attached ones are journaled. A
    /// link is part of a cell's observable state, so readers of either end must
    /// re-run. One body for both axes, so the rule cannot be applied to one and
    /// forgotten on the other.
    fn reattach_moved_links(
        &mut self,
        sheet: u32,
        axis: Axis,
        target_line: i32,
        moved_links: Vec<(i32, Link)>,
    ) -> Result<(), String> {
        let on_target = |&(row, column): &(i32, i32)| match axis {
            Axis::Row => row == target_line,
            Axis::Column => column == target_line,
        };
        let worksheet = self.workbook.worksheet_mut(sheet)?;
        let discarded: Vec<(i32, i32)> = worksheet
            .links
            .keys()
            .filter(|key| on_target(key))
            .copied()
            .collect();
        for (row, column) in discarded {
            worksheet.write_log.push(crate::recalc::Write::Link {
                at: (sheet, row, column),
            });
        }
        worksheet.links.retain(|key, _| !on_target(key));
        for (other, link) in moved_links {
            let (row, column) = match axis {
                Axis::Row => (target_line, other),
                Axis::Column => (other, target_line),
            };
            worksheet.links.insert((row, column), link);
            worksheet.write_log.push(crate::recalc::Write::Link {
                at: (sheet, row, column),
            });
        }
        Ok(())
    }

    /// Keeps the dependency graph in step with a structural edit. A row or
    /// column insert, delete or move shifts stored positions and formula
    /// `HYPERLINK` results. A cell displacement, which the shift does not model,
    /// forces a full recompute. The match is exhaustive so a new `DisplaceData`
    /// variant cannot silently skip graph maintenance.
    fn record_structural_edit(&mut self, disp: &DisplaceData) {
        // Inserted or deleted rows and columns add or remove formula cells
        // without cell writes; recount before the next fanout decision. CSE
        // rectangles move with their anchors, so the memo is stale too.
        self.formula_count_stale = true;
        self.cse_rects = None;
        match *disp {
            DisplaceData::Row { sheet, row, delta } => {
                self.record_band_edit(sheet, Axis::Row, row, delta)
            }
            DisplaceData::Column {
                sheet,
                column,
                delta,
            } => self.record_band_edit(sheet, Axis::Column, column, delta),
            // One line at a time: `move_rows_action` decomposes a K-row move
            // into K of these, so `row + delta` is the whole destination.
            DisplaceData::RowMove { sheet, row, delta } => {
                self.record_line_move(sheet, Axis::Row, row, row + delta)
            }
            DisplaceData::ColumnMove {
                sheet,
                column,
                delta,
            } => self.record_line_move(sheet, Axis::Column, column, column + delta),
            DisplaceData::CellHorizontal { .. } | DisplaceData::CellVertical { .. } => {
                self.graph.force_full();
            }
            DisplaceData::None => {}
        }
    }

    /// A row/column insert or delete: shift the dynamic links, then the graph.
    fn record_band_edit(&mut self, sheet: u32, axis: Axis, boundary: i32, delta: i32) {
        self.shift_dynamic_links(|pos| {
            crate::dependency_graph::shift_position(sheet, axis, boundary, delta, pos)
        });
        self.graph.structural_edit(sheet, axis, boundary, delta);
    }

    /// A row/column move: shift the dynamic links, then the graph.
    fn record_line_move(&mut self, sheet: u32, axis: Axis, from: i32, to: i32) {
        self.shift_dynamic_links(|pos| {
            crate::dependency_graph::move_position(sheet, axis, from, to, pos)
        });
        self.graph.structural_move(sheet, axis, from, to);
    }

    /// Moves formula `HYPERLINK` results with their cells, by the same rule the
    /// graph shifts itself by. Worksheet links are displaced separately; this
    /// map is not on the graph.
    fn shift_dynamic_links(
        &mut self,
        remap: impl Fn(crate::dependency_graph::Position) -> Option<crate::dependency_graph::Position>,
    ) {
        self.links = std::mem::take(&mut self.links)
            .into_iter()
            .filter_map(|(pos, link)| remap(pos).map(|pos| (pos, link)))
            .collect();
    }

    /// Inserts one or more new columns into the model at the specified index.
    ///
    /// This method shifts existing columns to the right to make space for the new columns.
    ///
    /// # Arguments
    ///
    /// * `sheet` - The sheet number to retrieve columns from.
    /// * `column` - The index at which the new columns should be inserted.
    /// * `column_count` - The number of columns to insert.
    pub fn insert_columns(
        &mut self,
        sheet: u32,
        column: i32,
        column_count: i32,
    ) -> Result<(), String> {
        if column_count <= 0 {
            return Err("Cannot add a negative number of cells :)".to_string());
        }
        if !self.can_insert_columns(sheet, column, column_count)? {
            return Err(
                "Cannot insert columns because that would break an array formula".to_string(),
            );
        }
        // check if it is possible:
        let dimensions = self.workbook.worksheet(sheet)?.dimension();
        let last_column = dimensions.max_column + column_count;
        if last_column > LAST_COLUMN {
            return Err(
                "Cannot shift cells because that would delete cells at the end of a row"
                    .to_string(),
            );
        }
        self.reset_dynamic_array_spills(sheet, Axis::Column, column)?;
        let worksheet = self.workbook.worksheet(sheet)?;
        let mut all_rows: Vec<i32> = worksheet.sheet_data.keys().copied().collect();
        all_rows.sort_unstable();
        for row in all_rows {
            let sorted_columns = self.get_columns_for_row(sheet, row, true)?;
            for col in sorted_columns {
                if col >= column {
                    self.move_cell(sheet, row, col, row, col + column_count)?;
                } else {
                    // Break because columns are in descending order.
                    break;
                }
            }
        }

        // Links move with their cells
        displace_links(self.workbook.worksheet_mut(sheet)?, sheet, |r, c| {
            if c >= column {
                Some((r, c + column_count))
            } else {
                Some((r, c))
            }
        });

        // Update all formulas in the workbook
        let disp = DisplaceData::Column {
            sheet,
            column,
            delta: column_count,
        };
        self.displace_cells(&disp)?;
        self.displace_cf_ranges(sheet, &disp);
        self.record_structural_edit(&disp);

        // In the list of columns:
        // * Keep all the columns to the left
        // * Displace all the columns to the right

        let worksheet = &mut self.workbook.worksheet_mut(sheet)?;

        let mut new_columns = Vec::new();
        for col in worksheet.cols.iter_mut() {
            // range under study
            let min = col.min;
            let max = col.max;
            if column > max {
                // If the range under study is to our left, this is a noop
            } else if column <= min {
                // If the range under study is to our right, we displace it
                col.min = min + column_count;
                col.max = max + column_count;
            } else {
                // If the range under study is in the middle we augment it
                col.max = max + column_count;
            }
            new_columns.push(col.clone());
        }
        // TODO: If in a row the cell to the right and left have the same style we should copy it

        worksheet.cols = new_columns;

        Ok(())
    }

    /// Deletes one or more columns from the model starting at the specified index.
    ///
    /// # Arguments
    ///
    /// * `sheet` - The sheet number to retrieve columns from.
    /// * `column` - The index of the first column to delete.
    /// * `count` - The number of columns to delete.
    pub fn delete_columns(
        &mut self,
        sheet: u32,
        column: i32,
        column_count: i32,
    ) -> Result<(), String> {
        if column_count <= 0 {
            return Err("Please use insert columns instead".to_string());
        }
        if !(1..=LAST_COLUMN).contains(&column) {
            return Err(format!("Column number '{column}' is not valid."));
        }
        if column + column_count - 1 > LAST_COLUMN {
            return Err("Cannot delete columns beyond the last column of the sheet".to_string());
        }
        if !self.can_delete_columns(sheet, column, column_count)? {
            return Err(
                "Cannot delete columns because that would break an array formula".to_string(),
            );
        }

        self.reset_dynamic_array_spills(sheet, Axis::Column, column)?;
        // first column being deleted
        let column_start = column;
        // last column being deleted
        let column_end = column + column_count - 1;

        // Move cells
        let worksheet = &self.workbook.worksheet(sheet)?;
        let mut all_rows: Vec<i32> = worksheet.sheet_data.keys().copied().collect();
        // We do not need to do that, but it is safer to eliminate sources of randomness in the algorithm
        all_rows.sort_unstable();

        for r in all_rows {
            let columns: Vec<i32> = self.get_columns_for_row(sheet, r, false)?;
            for col in columns {
                if col >= column_start {
                    if col > column_end {
                        self.move_cell(sheet, r, col, r, col - column_count)?;
                    } else {
                        // Sanctioned: the column itself is being deleted, so its cells lose
                        // content and style alike.
                        #[allow(clippy::disallowed_methods)]
                        self.workbook.worksheet_mut(sheet)?.remove_cell(r, col)?;
                    }
                }
            }
        }
        // Links move with their cells; the links of the deleted columns are removed
        displace_links(self.workbook.worksheet_mut(sheet)?, sheet, |r, c| {
            if c < column_start {
                Some((r, c))
            } else if c <= column_end {
                None
            } else {
                Some((r, c - column_count))
            }
        });

        // Update all formulas in the workbook
        let disp = DisplaceData::Column {
            sheet,
            column,
            delta: -column_count,
        };
        self.displace_cells(&disp)?;
        self.displace_cf_ranges(sheet, &disp);
        self.record_structural_edit(&disp);
        let worksheet = &mut self.workbook.worksheet_mut(sheet)?;

        // deletes all the column styles
        let mut new_columns = Vec::new();
        for col in worksheet.cols.iter_mut() {
            // range under study
            let min = col.min;
            let max = col.max;
            // In the diagram:
            // |xxxxx| range we are studying [min, max]
            // |*****| range we are deleting [column_start, column_end]
            // we are going to split it in three big cases:
            // ----------------|xxxxxxxx|-----------------
            // -----|*****|------------------------------- Case A
            // -------|**********|------------------------ Case B
            // -------------|**************|-------------- Case C
            // ------------------|****|------------------- Case D
            // ---------------------|**********|---------- Case E
            // -----------------------------|*****|------- Case F
            if column_start < min {
                if column_end < min {
                    // Case A
                    // We displace all columns
                    let mut new_column = col.clone();
                    new_column.min = min - column_count;
                    new_column.max = max - column_count;
                    new_columns.push(new_column);
                } else if column_end < max {
                    // Case B
                    // We displace the end
                    let mut new_column = col.clone();
                    new_column.min = column_start;
                    new_column.max = max - column_count;
                    new_columns.push(new_column);
                } else {
                    // Case C
                    // skip this, we are deleting the whole range
                }
            } else if column_start <= max {
                if column_end <= max {
                    // Case D
                    // We displace the end
                    let mut new_column = col.clone();
                    new_column.max = max - column_count;
                    new_columns.push(new_column);
                } else {
                    // Case E
                    let mut new_column = col.clone();
                    new_column.max = column_start - 1;
                    new_columns.push(new_column);
                }
            } else {
                // Case F
                // No action required
                new_columns.push(col.clone());
            }
        }
        worksheet.cols = new_columns;

        Ok(())
    }

    // Returns true if inserting rows at `row` would not split any array formula.
    // Inserting at `row` shifts every row >= `row` down. A formula whose anchor
    // row is strictly above `row` but whose spill extends to `row` or below would
    // be split, so we must reject that.
    fn can_insert_rows(&self, sheet: u32, row: i32, _row_count: i32) -> Result<bool, String> {
        let cell_coords: Vec<(i32, i32)> = {
            let worksheet = self.workbook.worksheet(sheet)?;
            worksheet
                .sheet_data
                .iter()
                .flat_map(|(r, row_data)| row_data.keys().map(move |c| (*r, *c)))
                .collect()
        };
        for (r, c) in cell_coords {
            if let CellStructure::ArrayFormula { range: (_, height) } =
                self.get_cell_structure(sheet, r, c)?
            {
                // The formula spans rows [r, r + height - 1].
                // Inserting at `row` splits it when the anchor is above `row`
                // but the spill reaches `row` or beyond.
                if r < row && row < r + height {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    // Returns true if inserting columns at `column` would not split any array formula.
    fn can_insert_columns(
        &self,
        sheet: u32,
        column: i32,
        _column_count: i32,
    ) -> Result<bool, String> {
        let cell_coords: Vec<(i32, i32)> = {
            let worksheet = self.workbook.worksheet(sheet)?;
            worksheet
                .sheet_data
                .iter()
                .flat_map(|(r, row_data)| row_data.keys().map(move |c| (*r, *c)))
                .collect()
        };
        for (r, c) in cell_coords {
            if let CellStructure::ArrayFormula { range: (width, _) } =
                self.get_cell_structure(sheet, r, c)?
            {
                if c < column && column < c + width {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    // Returns true if deleting rows [row, row + row_count - 1] would not break any
    // array formula. An array formula must be either fully inside the deleted range
    // or fully outside it; any partial overlap is rejected.
    fn can_delete_rows(&self, sheet: u32, row: i32, row_count: i32) -> Result<bool, String> {
        let row_end = row + row_count; // exclusive upper bound
        let cell_coords: Vec<(i32, i32)> = {
            let worksheet = self.workbook.worksheet(sheet)?;
            worksheet
                .sheet_data
                .iter()
                .flat_map(|(r, row_data)| row_data.keys().map(move |c| (*r, *c)))
                .collect()
        };
        for (r, c) in cell_coords {
            if let CellStructure::ArrayFormula { range: (_, height) } =
                self.get_cell_structure(sheet, r, c)?
            {
                // Formula row span: [r, r + height - 1]
                let overlaps = r < row_end && r + height > row;
                let contained = r >= row && r + height <= row_end;
                if overlaps && !contained {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    // Returns true if deleting columns [column, column + column_count - 1] would not
    // break any array formula.
    fn can_delete_columns(
        &self,
        sheet: u32,
        column: i32,
        column_count: i32,
    ) -> Result<bool, String> {
        let col_end = column + column_count; // exclusive upper bound
        let cell_coords: Vec<(i32, i32)> = {
            let worksheet = self.workbook.worksheet(sheet)?;
            worksheet
                .sheet_data
                .iter()
                .flat_map(|(r, row_data)| row_data.keys().map(move |c| (*r, *c)))
                .collect()
        };
        for (r, c) in cell_coords {
            if let CellStructure::ArrayFormula { range: (width, _) } =
                self.get_cell_structure(sheet, r, c)?
            {
                // Formula column span: [c, c + width - 1]
                let overlaps = c < col_end && c + width > column;
                let contained = c >= column && c + width <= col_end;
                if overlaps && !contained {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// Inserts one or more new rows into the model at the specified index.
    ///
    /// # Arguments
    ///
    /// * `sheet` - The sheet number to retrieve columns from.
    /// * `row` - The index at which the new rows should be inserted.
    /// * `row_count` - The number of rows to insert.
    pub fn insert_rows(&mut self, sheet: u32, row: i32, row_count: i32) -> Result<(), String> {
        if row_count <= 0 {
            return Err("Cannot add a negative number of cells :)".to_string());
        }
        if !self.can_insert_rows(sheet, row, row_count)? {
            return Err("Cannot insert rows because that would break an array formula".to_string());
        }
        // Check if it is possible:
        let dimensions = self.workbook.worksheet(sheet)?.dimension();
        let last_row = dimensions.max_row + row_count;
        if last_row > LAST_ROW {
            return Err(
                "Cannot shift cells because that would delete cells at the end of a column"
                    .to_string(),
            );
        }

        self.reset_dynamic_array_spills(sheet, Axis::Row, row)?;
        // Move cells
        let worksheet = &self.workbook.worksheet(sheet)?;
        let mut all_rows: Vec<i32> = worksheet.sheet_data.keys().copied().collect();
        all_rows.sort_unstable();
        all_rows.reverse();
        for r in all_rows {
            if r >= row {
                // We do not really need the columns in any order
                let columns: Vec<i32> = self.get_columns_for_row(sheet, r, false)?;
                for column in columns {
                    self.move_cell(sheet, r, column, r + row_count, column)?;
                }
            } else {
                // Rows are in descending order
                break;
            }
        }
        // In the list of rows styles:
        // * Add all rows above the rows we are inserting unchanged
        // * Shift the ones below
        let rows = &self.workbook.worksheets[sheet as usize].rows;
        let mut new_rows = vec![];
        for r in rows {
            if r.r < row {
                new_rows.push(r.clone());
            } else if r.r >= row {
                let mut new_row = r.clone();
                new_row.r = r.r + row_count;
                new_rows.push(new_row);
            }
        }
        self.workbook.worksheets[sheet as usize].rows = new_rows;

        // Links move with their cells
        displace_links(self.workbook.worksheet_mut(sheet)?, sheet, |r, c| {
            if r >= row {
                Some((r + row_count, c))
            } else {
                Some((r, c))
            }
        });

        // Update all formulas in the workbook
        let disp = DisplaceData::Row {
            sheet,
            row,
            delta: row_count,
        };
        self.displace_cells(&disp)?;
        self.displace_cf_ranges(sheet, &disp);
        self.record_structural_edit(&disp);

        Ok(())
    }

    /// Deletes one or more rows from the model starting at the specified index.
    ///
    /// # Arguments
    ///
    /// * `sheet` - The sheet number to retrieve columns from.
    /// * `row` - The index of the first row to delete.
    /// * `row_count` - The number of rows to delete.
    pub fn delete_rows(&mut self, sheet: u32, row: i32, row_count: i32) -> Result<(), String> {
        if row_count <= 0 {
            return Err("Please use insert rows instead".to_string());
        }
        if !(1..=LAST_ROW).contains(&row) {
            return Err(format!("Row number '{row}' is not valid."));
        }
        if row + row_count - 1 > LAST_ROW {
            return Err("Cannot delete rows beyond the last row of the sheet".to_string());
        }
        if !self.can_delete_rows(sheet, row, row_count)? {
            return Err("Cannot delete rows because that would break an array formula".to_string());
        }

        self.reset_dynamic_array_spills(sheet, Axis::Row, row)?;
        // Move cells
        let worksheet = &self.workbook.worksheet(sheet)?;
        let mut all_rows: Vec<i32> = worksheet.sheet_data.keys().copied().collect();
        all_rows.sort_unstable();

        for r in all_rows {
            if r >= row {
                // We do not need ordered, but it is safer to eliminate sources of randomness in the algorithm
                let columns: Vec<i32> = self.get_columns_for_row(sheet, r, false)?;
                if r >= row + row_count {
                    // displace all cells in column
                    for column in columns {
                        self.move_cell(sheet, r, column, r - row_count, column)?;
                    }
                } else {
                    // remove all cells in row
                    self.workbook.worksheet_mut(sheet)?.remove_row_data(r);
                }
            }
        }
        // In the list of rows styles:
        // * Add all rows above the rows we are deleting unchanged
        // * Skip all those we are deleting
        // * Shift the ones below
        let rows = &self.workbook.worksheets[sheet as usize].rows;
        let mut new_rows = vec![];
        for r in rows {
            if r.r < row {
                new_rows.push(r.clone());
            } else if r.r >= row + row_count {
                let mut new_row = r.clone();
                new_row.r = r.r - row_count;
                new_rows.push(new_row);
            }
        }
        self.workbook.worksheets[sheet as usize].rows = new_rows;

        // Links move with their cells; the links of the deleted rows are removed
        displace_links(self.workbook.worksheet_mut(sheet)?, sheet, |r, c| {
            if r < row {
                Some((r, c))
            } else if r < row + row_count {
                None
            } else {
                Some((r - row_count, c))
            }
        });

        let disp = DisplaceData::Row {
            sheet,
            row,
            delta: -row_count,
        };
        self.displace_cells(&disp)?;
        self.displace_cf_ranges(sheet, &disp);
        self.record_structural_edit(&disp);
        Ok(())
    }

    // Inner column move: no boundary/can check, no spill reset.
    // Caller must have validated and reset spills before calling this.
    fn move_column_unchecked(&mut self, sheet: u32, column: i32, delta: i32) -> Result<(), String> {
        let target_column = column + delta;

        // Links move with their cells: take the moved column's links out and
        // shift the links of the columns in between. The moved links are
        // re-attached at the end, after the cells have been rebuilt (rebuilding
        // goes through `set_user_input`, which could auto-link URL-like values).
        let worksheet = self.workbook.worksheet_mut(sheet)?;
        let moved_links: Vec<(i32, Link)> = worksheet
            .links
            .iter()
            .filter(|(&(_, c), _)| c == column)
            .map(|(&(r, _), link)| (r, link.clone()))
            .collect();
        displace_links(worksheet, sheet, |r, c| {
            if c == column {
                None
            } else if delta > 0 && c > column && c <= target_column {
                Some((r, c - 1))
            } else if delta < 0 && c >= target_column && c < column {
                Some((r, c + 1))
            } else {
                Some((r, c))
            }
        });

        let original_refs = self
            .workbook
            .worksheet(sheet)?
            .column_cell_references(column)?;
        let mut original_cells = Vec::new();
        for r in &original_refs {
            let cell = self
                .workbook
                .worksheet(sheet)?
                .cell(r.row, column)
                .ok_or("Expected Cell to exist")?;
            let style_idx = cell.get_style();
            let formula_or_value =
                self.get_cell_formula(sheet, r.row, column)?
                    .unwrap_or_else(|| {
                        cell.get_localized_text(
                            &self.workbook.shared_strings,
                            self.locale,
                            self.language,
                        )
                    });

            let mut array = None;

            match cell {
                Cell::EmptyCell { .. }
                | Cell::BooleanCell { .. }
                | Cell::NumberCell { .. }
                | Cell::ErrorCell { .. }
                | Cell::SharedString { .. }
                | Cell::CellFormula { .. } => {
                    // This is a regular cell, we can just move it.
                }
                Cell::SpillCell { .. } => {
                    // This the spill of an array formula. Because dynamic arrays spills have been deleted
                    // We delete the spill
                    let worksheet = self.workbook.worksheet_mut(sheet)?;
                    // Sanctioned: vacating the source of a column move. The position is
                    // meant to lose its style too, unlike a footprint teardown.
                    #[allow(clippy::disallowed_methods)]
                    worksheet.remove_cell(r.row, column)?;
                    continue;
                }
                Cell::ArrayFormula {
                    r,
                    kind: ArrayKind::Dynamic,
                    ..
                } => {
                    // We are moving the anchor of a dynamic formula.
                    // We assume the spill has been taken care of by the caller
                    debug_assert_eq!(*r, (1, 1));
                }
                Cell::ArrayFormula {
                    r,
                    kind: ArrayKind::Cse,
                    ..
                } => {
                    // This is an array formula, we need to move the whole range
                    // We rely on the calling function to check that the move is valid and does not split the array formula
                    array = Some(*r);
                }
            }

            original_cells.push((r.row, formula_or_value, style_idx, array));
            let ws = self.workbook.worksheet_mut(sheet)?;
            // Sanctioned: vacating the source of a column move; the cell is
            // re-created at the target with its captured style.
            #[allow(clippy::disallowed_methods)]
            ws.remove_cell(r.row, column)?;
        }
        let width = self
            .workbook
            .worksheet(sheet)?
            .get_actual_column_width(column)?;
        let style = self.workbook.worksheet(sheet)?.get_column_style(column)?;
        let hidden = self.workbook.worksheet(sheet)?.is_column_hidden(column)?;
        if delta > 0 {
            for c in column + 1..=target_column {
                let refs = self.workbook.worksheet(sheet)?.column_cell_references(c)?;
                for r in refs {
                    self.move_cell(sheet, r.row, c, r.row, c - 1)?;
                }
                let w = self.workbook.worksheet(sheet)?.get_actual_column_width(c)?;
                let s = self.workbook.worksheet(sheet)?.get_column_style(c)?;
                let h = self.workbook.worksheet(sheet)?.is_column_hidden(c)?;
                self.workbook
                    .worksheet_mut(sheet)?
                    .set_column_width_and_style(c - 1, w, h, s)?;
            }
        } else {
            for c in (target_column..=column - 1).rev() {
                let refs = self.workbook.worksheet(sheet)?.column_cell_references(c)?;
                for r in refs {
                    self.move_cell(sheet, r.row, c, r.row, c + 1)?;
                }
                let w = self.workbook.worksheet(sheet)?.get_actual_column_width(c)?;
                let s = self.workbook.worksheet(sheet)?.get_column_style(c)?;
                let h = self.workbook.worksheet(sheet)?.is_column_hidden(c)?;
                self.workbook
                    .worksheet_mut(sheet)?
                    .set_column_width_and_style(c + 1, w, h, s)?;
            }
        }
        let rebuilt: Vec<MovedCell> = original_cells
            .into_iter()
            .map(|(r, value, style_idx, array)| (r, target_column, value, style_idx, array))
            .collect();
        self.with_cse_guard_suspended(|model| model.rebuild_moved_cells(sheet, rebuilt))?;
        self.workbook
            .worksheet_mut(sheet)?
            .set_column_width_and_style(target_column, width, hidden, style)?;

        self.reattach_moved_links(sheet, Axis::Column, target_column, moved_links)?;

        let disp = DisplaceData::ColumnMove {
            sheet,
            column,
            delta,
        };
        self.displace_cells(&disp)?;
        self.displace_cf_ranges(sheet, &disp);
        self.record_structural_edit(&disp);
        Ok(())
    }

    // Inner row move: no boundary/can check, no spill reset.
    fn move_row_unchecked(&mut self, sheet: u32, row: i32, delta: i32) -> Result<(), String> {
        let target_row = row + delta;

        // Links move with their cells: take the moved row's links out and shift
        // the links of the rows in between. The moved links are re-attached at
        // the end, after the cells have been rebuilt (rebuilding goes through
        // `set_user_input`, which could auto-link URL-like values).
        let worksheet = self.workbook.worksheet_mut(sheet)?;
        let moved_links: Vec<(i32, Link)> = worksheet
            .links
            .iter()
            .filter(|(&(r, _), _)| r == row)
            .map(|(&(_, c), link)| (c, link.clone()))
            .collect();
        displace_links(worksheet, sheet, |r, c| {
            if r == row {
                None
            } else if delta > 0 && r > row && r <= target_row {
                Some((r - 1, c))
            } else if delta < 0 && r >= target_row && r < row {
                Some((r + 1, c))
            } else {
                Some((r, c))
            }
        });

        let original_cols = self.get_columns_for_row(sheet, row, false)?;
        let mut original_cells = Vec::new();
        for c in &original_cols {
            let cell = self
                .workbook
                .worksheet(sheet)?
                .cell(row, *c)
                .ok_or("Expected Cell to exist")?;
            let style_idx = cell.get_style();
            let formula_or_value = self.get_cell_formula(sheet, row, *c)?.unwrap_or_else(|| {
                cell.get_localized_text(&self.workbook.shared_strings, self.locale, self.language)
            });
            let mut array = None;

            match cell {
                Cell::EmptyCell { .. }
                | Cell::BooleanCell { .. }
                | Cell::NumberCell { .. }
                | Cell::ErrorCell { .. }
                | Cell::SharedString { .. }
                | Cell::CellFormula { .. } => {
                    // This is a regular cell, we can just move it.
                }
                Cell::SpillCell { .. } => {
                    // This the spill of an array formula. Because dynamic arrays spills have been deleted
                    // We delete the spill
                    let worksheet = self.workbook.worksheet_mut(sheet)?;
                    // Sanctioned: vacating the source of a row move. The position is meant
                    // to lose its style too, unlike a footprint teardown.
                    #[allow(clippy::disallowed_methods)]
                    worksheet.remove_cell(row, *c)?;
                    continue;
                }
                Cell::ArrayFormula {
                    r,
                    kind: ArrayKind::Dynamic,
                    ..
                } => {
                    // We are moving the anchor of a dynamic formula.
                    // We assume the spill has been taken care of by the caller
                    debug_assert_eq!(*r, (1, 1));
                }
                Cell::ArrayFormula {
                    r,
                    kind: ArrayKind::Cse,
                    ..
                } => {
                    // This is an array formula, we need to move the whole range
                    // We rely on the calling function to check that the move is valid and does not split the array formula
                    array = Some(*r);
                }
            }
            original_cells.push((*c, formula_or_value, style_idx, array));
            let ws = self.workbook.worksheet_mut(sheet)?;
            // Sanctioned: vacating the source of a row move; the cell is re-created
            // at the target with its captured style.
            #[allow(clippy::disallowed_methods)]
            ws.remove_cell(row, *c)?;
        }
        if delta > 0 {
            for r in row + 1..=target_row {
                let cols = self.get_columns_for_row(sheet, r, false)?;
                for c in cols {
                    self.move_cell(sheet, r, c, r - 1, c)?;
                }
            }
        } else {
            for r in (target_row..=row - 1).rev() {
                let cols = self.get_columns_for_row(sheet, r, false)?;
                for c in cols {
                    self.move_cell(sheet, r, c, r + 1, c)?;
                }
            }
        }
        let rebuilt: Vec<MovedCell> = original_cells
            .into_iter()
            .map(|(c, value, style_idx, array)| (target_row, c, value, style_idx, array))
            .collect();
        self.with_cse_guard_suspended(|model| model.rebuild_moved_cells(sheet, rebuilt))?;
        let worksheet = &mut self.workbook.worksheet_mut(sheet)?;
        let mut new_rows = Vec::new();
        for r in worksheet.rows.iter() {
            if r.r == row {
                let mut nr = r.clone();
                nr.r = target_row;
                new_rows.push(nr);
            } else if delta > 0 && r.r > row && r.r <= target_row {
                let mut nr = r.clone();
                nr.r -= 1;
                new_rows.push(nr);
            } else if delta < 0 && r.r < row && r.r >= target_row {
                let mut nr = r.clone();
                nr.r += 1;
                new_rows.push(nr);
            } else {
                new_rows.push(r.clone());
            }
        }
        worksheet.rows = new_rows;

        self.reattach_moved_links(sheet, Axis::Row, target_row, moved_links)?;

        let disp = DisplaceData::RowMove { sheet, row, delta };
        self.displace_cells(&disp)?;
        self.displace_cf_ranges(sheet, &disp);
        self.record_structural_edit(&disp);
        Ok(())
    }

    // Returns true if moving columns [column, column+column_count-1] by delta would not
    // split any CSE array formula. A formula is OK if its column span is fully within
    // the moved group, fully within the displaced zone, or fully outside both.
    fn can_move_columns_action(
        &self,
        sheet: u32,
        column: i32,
        column_count: i32,
        delta: i32,
    ) -> Result<bool, String> {
        if delta == 0 {
            return Ok(true);
        }

        let group_start = column;
        let group_end = column + column_count - 1;

        let (displace_start, displace_end) = if delta > 0 {
            (group_end + 1, group_end + delta)
        } else {
            (group_start + delta, group_start - 1)
        };

        let overlaps = |a_start: i32, a_end: i32, b_start: i32, b_end: i32| {
            a_start <= b_end && b_start <= a_end
        };

        let contains = |a_start: i32, a_end: i32, b_start: i32, b_end: i32| {
            a_start <= b_start && b_end <= a_end
        };

        let interval_is_safe = |array_start: i32, array_end: i32| {
            let safe_for = |start: i32, end: i32| {
                !overlaps(start, end, array_start, array_end)
                    || contains(start, end, array_start, array_end)
            };
            safe_for(group_start, group_end) && safe_for(displace_start, displace_end)
        };

        let cell_coords: Vec<(i32, i32)> = {
            let worksheet = self.workbook.worksheet(sheet)?;
            worksheet
                .sheet_data
                .iter()
                .flat_map(|(r, row_data)| row_data.keys().map(move |c| (*r, *c)))
                .collect()
        };

        for (r, c) in cell_coords {
            match self.get_cell_structure(sheet, r, c)? {
                CellStructure::ArrayFormula { range } => {
                    let (width, _) = range;
                    let array_start_col = c;
                    let array_end_col = c + width - 1;

                    if !interval_is_safe(array_start_col, array_end_col) {
                        return Ok(false);
                    }
                }
                CellStructure::SpillArray { anchor, range } => {
                    let (width, _) = range;
                    let (_, array_start_col) = anchor;
                    let array_end_col = array_start_col + width - 1;

                    if !interval_is_safe(array_start_col, array_end_col) {
                        return Ok(false);
                    }
                }
                _ => {}
            }
        }

        Ok(true)
    }

    // Returns true if moving rows [row, row+row_count-1] by delta would not
    // split any CSE array formula.
    // That could happen because:
    // * rows are moved in the middle of an array formula
    // * we move part of an array
    fn can_move_rows_action(
        &self,
        sheet: u32,
        row: i32,
        row_count: i32,
        delta: i32,
    ) -> Result<bool, String> {
        if delta == 0 {
            return Ok(true);
        }

        let group_start = row;
        let group_end = row + row_count - 1;

        let (displace_start, displace_end) = if delta > 0 {
            (group_end + 1, group_end + delta)
        } else {
            (group_start + delta, group_start - 1)
        };

        let overlaps = |a_start: i32, a_end: i32, b_start: i32, b_end: i32| {
            a_start <= b_end && b_start <= a_end
        };

        let contains = |a_start: i32, a_end: i32, b_start: i32, b_end: i32| {
            a_start <= b_start && b_end <= a_end
        };

        let interval_is_safe = |array_start: i32, array_end: i32| {
            let safe_for = |start: i32, end: i32| {
                !overlaps(start, end, array_start, array_end)
                    || contains(start, end, array_start, array_end)
            };

            safe_for(group_start, group_end) && safe_for(displace_start, displace_end)
        };

        // list of all the cells in the sheet
        let cell_coords: Vec<(i32, i32)> = {
            let worksheet = self.workbook.worksheet(sheet)?;
            worksheet
                .sheet_data
                .iter()
                .flat_map(|(r, row_data)| row_data.keys().map(move |c| (*r, *c)))
                .collect()
        };

        for (r, c) in cell_coords {
            match self.get_cell_structure(sheet, r, c)? {
                CellStructure::ArrayFormula { range } => {
                    let (_, height) = range;
                    let array_start_row = r;
                    let array_end_row = r + height - 1;

                    if !interval_is_safe(array_start_row, array_end_row) {
                        return Ok(false);
                    }
                }
                CellStructure::SpillArray { anchor, range } => {
                    let (_, height) = range;
                    let (array_start_row, _) = anchor;
                    let array_end_row = array_start_row + height - 1;

                    if !interval_is_safe(array_start_row, array_end_row) {
                        return Ok(false);
                    }
                }
                _ => {}
            }
        }

        Ok(true)
    }

    /// Moves a group of columns [column, column+column_count-1] by delta positions.
    /// CSE array formulas fully within the moved group are preserved as arrays.
    /// Displaces cells due to a move column action
    /// from initial_column to target_column = initial_column + column_delta
    /// References will be updated following:
    /// Cell references:
    ///    * All cell references to initial_column will go to target_column
    ///    * All cell references to columns in between (initial_column, target_column] will be displaced one to the left
    ///    * All other cell references are left unchanged
    ///      Ranges. This is the tricky bit:
    ///    * Column is one of the extremes of the range. The new extreme would be target_column.
    ///      Range is then normalized
    ///    * Any other case, range is left unchanged.
    ///      NOTE: This moves the data and column styles along with the formulas
    pub fn move_columns_action(
        &mut self,
        sheet: u32,
        column: i32,
        column_count: i32,
        delta: i32,
    ) -> Result<(), String> {
        if column_count <= 0 || delta == 0 {
            return Ok(());
        }
        let target_first = column + delta;
        let target_last = column + column_count - 1 + delta;
        if !(1..=LAST_COLUMN).contains(&target_first) || !(1..=LAST_COLUMN).contains(&target_last) {
            return Err("Target column out of boundaries".to_string());
        }
        if !(1..=LAST_COLUMN).contains(&column)
            || !(1..=LAST_COLUMN).contains(&(column + column_count - 1))
        {
            return Err("Initial column out of boundaries".to_string());
        }
        if !self.can_move_columns_action(sheet, column, column_count, delta)? {
            return Err(
                "Cannot move columns because that would split an array formula".to_string(),
            );
        }
        self.reset_dynamic_array_spills(sheet, Axis::Column, 1)?;

        // Move columns in the correct order
        if delta > 0 {
            for col in (column..column + column_count).rev() {
                self.move_column_unchecked(sheet, col, delta)?;
            }
        } else {
            for col in column..column + column_count {
                self.move_column_unchecked(sheet, col, delta)?;
            }
        }

        Ok(())
    }

    /// Displaces cells due to a move row action
    /// from initial_row to target_row = initial_row + row_delta
    /// References will be updated following the same rules as move_column_action
    /// NOTE: This moves the data and row styles along with the formulas
    /// Moves a group of rows [row, row+row_count-1] by delta positions.
    /// CSE array formulas fully within the moved group are preserved as arrays.
    pub fn move_rows_action(
        &mut self,
        sheet: u32,
        row: i32,
        row_count: i32,
        delta: i32,
    ) -> Result<(), String> {
        if row_count <= 0 || delta == 0 {
            return Ok(());
        }
        let target_first = row + delta;
        let target_last = row + row_count - 1 + delta;
        if !(1..=LAST_ROW).contains(&target_first) || !(1..=LAST_ROW).contains(&target_last) {
            return Err("Target row out of boundaries".to_string());
        }
        if !(1..=LAST_ROW).contains(&row) || !(1..=LAST_ROW).contains(&(row + row_count - 1)) {
            return Err("Initial row out of boundaries".to_string());
        }
        if !self.can_move_rows_action(sheet, row, row_count, delta)? {
            return Err("Cannot move rows because that would split an array formula".to_string());
        }
        self.reset_dynamic_array_spills(sheet, Axis::Row, 1)?;

        // Move rows in the correct order
        if delta > 0 {
            for r in (row..row + row_count).rev() {
                self.move_row_unchecked(sheet, r, delta)?;
            }
        } else {
            for r in row..row + row_count {
                self.move_row_unchecked(sheet, r, delta)?;
            }
        }
        Ok(())
    }
}
