use pyo3::exceptions::PyException;
use pyo3::{create_exception, prelude::*, wrap_pyfunction};

use types::{PyCellType, PySheetProperty, PyStyle, PyDefinedName, PyArea};
use xlsx::base::types::{Style, Workbook};
use xlsx::base::{Model, UserModel};

use xlsx::export::{save_to_icalc, save_to_xlsx};
use xlsx::import;

mod types;

create_exception!(_ironcalc, WorkbookError, PyException);

#[pyclass]
pub struct PyUserModel {
    /// The user model, which is a wrapper around the Model
    pub model: UserModel,
}

#[pymethods]
impl PyUserModel {
    /// Saves the user model to an xlsx file
    pub fn save_to_xlsx(&self, file: &str) -> PyResult<()> {
        let model = self.model.get_model();
        save_to_xlsx(model, file).map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    /// Saves the user model to file in the internal binary ic format
    pub fn save_to_icalc(&self, file: &str) -> PyResult<()> {
        let model = self.model.get_model();
        save_to_icalc(model, file).map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn apply_external_diffs(&mut self, external_diffs: &[u8]) -> PyResult<()> {
        self.model
            .apply_external_diffs(external_diffs)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn flush_send_queue(&mut self) -> Vec<u8> {
        self.model.flush_send_queue()
    }

    // Editing operations / evaluation helpers
    pub fn undo(&mut self) -> PyResult<()> {
        self.model.undo().map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn redo(&mut self) -> PyResult<()> {
        self.model.redo().map_err(|e| WorkbookError::new_err(e.to_string()))
    }

        pub fn can_undo(&self) -> bool {
        self.model.can_undo()
    }

        pub fn can_redo(&self) -> bool {
        self.model.can_redo()
    }

    pub fn pause_evaluation(&mut self) {
        self.model.pause_evaluation();
    }

    pub fn resume_evaluation(&mut self) {
        self.model.resume_evaluation();
    }

    pub fn evaluate(&mut self) {
        self.model.evaluate();
    }

    pub fn get_cell_content(&self, sheet: u32, row: i32, column: i32) -> PyResult<String> {
        self.model
            .get_cell_content(sheet, row, column)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> PyResult<PyCellType> {
        self.model
            .get_cell_type(sheet, row, column)
            .map(|ct| ct.into())
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn move_row(&mut self, sheet: u32, row: i32, delta: i32) -> PyResult<()> {
        self.model
            .move_row_action(sheet, row, delta)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn move_column(&mut self, sheet: u32, column: i32, delta: i32) -> PyResult<()> {
        self.model
            .move_column_action(sheet, column, delta)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Defined names helpers - PyModel uses workbook directly
    pub fn get_defined_name_list(&self) -> Vec<PyDefinedName> {
        self.model
            .get_defined_name_list()            .iter()
            .map(|(name, scope, formula)| PyDefinedName { 
                name: name.to_owned(), 
                scope: *scope, 
                formula: formula.to_owned() 
            })
            .collect()
    }

    pub fn new_defined_name(
        &mut self,
        name: &str,
        scope: Option<u32>,
        formula: &str,
    ) -> PyResult<()> {
        self.model
            .new_defined_name(name, scope, formula)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn update_defined_name(
        &mut self,
        name: &str,
        scope: Option<u32>,
        new_name: &str,
        new_scope: Option<u32>,
        new_formula: &str,
    ) -> PyResult<()> {
        self.model
            .update_defined_name(name, scope, new_name, new_scope, new_formula)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn delete_defined_name(&mut self, name: &str, scope: Option<u32>) -> PyResult<()> {
        self.model
            .delete_defined_name(name, scope)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Sheet management
    pub fn new_sheet(&mut self) -> PyResult<()> {
        self.model.new_sheet().map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn delete_sheet(&mut self, sheet: u32) -> PyResult<()> {
        self.model.delete_sheet(sheet).map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn hide_sheet(&mut self, sheet: u32) -> PyResult<()> {
        self.model.hide_sheet(sheet).map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn unhide_sheet(&mut self, sheet: u32) -> PyResult<()> {
        self.model.unhide_sheet(sheet).map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn rename_sheet(&mut self, sheet: u32, name: &str) -> PyResult<()> {
        self.model.rename_sheet(sheet, name).map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn set_sheet_color(&mut self, sheet: u32, color: &str) -> PyResult<()> {
        self.model
            .set_sheet_color(sheet, color)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Range operations
    pub fn range_clear_all(&mut self, sheet: u32, start_row: i32, start_column: i32, end_row: i32, end_column: i32) -> PyResult<()> {
        use xlsx::base::expressions::types::Area;
        let range = Area {
            sheet,
            row: start_row,
            column: start_column,
            width: end_column - start_column + 1,
            height: end_row - start_row + 1,
        };
        self.model.range_clear_all(&range).map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn range_clear_contents(&mut self, sheet: u32, start_row: i32, start_column: i32, end_row: i32, end_column: i32) -> PyResult<()> {
        use xlsx::base::expressions::types::Area;
        let range = Area {
            sheet,
            row: start_row,
            column: start_column,
            width: end_column - start_column + 1,
            height: end_row - start_row + 1,
        };
        self.model.range_clear_contents(&range).map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn range_clear_formatting(&mut self, sheet: u32, start_row: i32, start_column: i32, end_row: i32, end_column: i32) -> PyResult<()> {
        use xlsx::base::expressions::types::Area;
        let range = Area {
            sheet,
            row: start_row,
            column: start_column,
            width: end_column - start_column + 1,
            height: end_row - start_row + 1,
        };
        self.model.range_clear_formatting(&range).map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Rows and columns operations
    pub fn insert_rows(&mut self, sheet: u32, row: i32, row_count: i32) -> PyResult<()> {
        self.model
            .insert_rows(sheet, row, row_count)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn insert_columns(&mut self, sheet: u32, column: i32, column_count: i32) -> PyResult<()> {
        self.model
            .insert_columns(sheet, column, column_count)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn delete_rows(&mut self, sheet: u32, row: i32, row_count: i32) -> PyResult<()> {
        self.model
            .delete_rows(sheet, row, row_count)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn delete_columns(&mut self, sheet: u32, column: i32, column_count: i32) -> PyResult<()> {
        self.model
            .delete_columns(sheet, column, column_count)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Height and width
    pub fn set_rows_height(&mut self, sheet: u32, row_start: i32, row_end: i32, height: f64) -> PyResult<()> {
        self.model
            .set_rows_height(sheet, row_start, row_end, height)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn set_columns_width(&mut self, sheet: u32, column_start: i32, column_end: i32, width: f64) -> PyResult<()> {
        self.model
            .set_columns_width(sheet, column_start, column_end, width)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn get_row_height(&mut self, sheet: u32, row: i32) -> PyResult<f64> {
        self.model.get_row_height(sheet, row).map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn get_column_width(&mut self, sheet: u32, column: i32) -> PyResult<f64> {
        self.model
            .get_column_width(sheet, column)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Frozen panes
    pub fn get_frozen_rows_count(&self, sheet: u32) -> PyResult<i32> {
        self.model.get_frozen_rows_count(sheet).map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn get_frozen_columns_count(&self, sheet: u32) -> PyResult<i32> {
        self.model
            .get_frozen_columns_count(sheet)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn set_frozen_rows_count(&mut self, sheet: u32, count: i32) -> PyResult<()> {
        self.model
            .set_frozen_rows_count(sheet, count)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn set_frozen_columns_count(&mut self, sheet: u32, count: i32) -> PyResult<()> {
        self.model
            .set_frozen_columns_count(sheet, count)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Styles
    pub fn update_range_style(&mut self, area: &PyArea, style_path: &str, value: &str) -> PyResult<()> {
        let range = area.clone().into();
        self.model
            .update_range_style(&range, style_path, value)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn get_cell_style(&mut self, sheet: u32, row: i32, column: i32) -> PyResult<PyStyle> {
        let style = self
            .model
            .get_cell_style(sheet, row, column)
            .map_err(|e| WorkbookError::new_err(e.to_string()))?;
        Ok(style.into())
    }

    pub fn on_paste_styles(&mut self, styles: Vec<Vec<PyStyle>>) -> PyResult<()> {
        let rust_styles: Vec<Vec<xlsx::base::types::Style>> = styles
            .into_iter()
            .map(|row| row.into_iter().map(|style| (&style).into()).collect())
            .collect();
        self.model.on_paste_styles(&rust_styles).map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn set_user_input(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        value: &str,
    ) -> PyResult<()> {
        self.model
            .set_user_input(sheet, row, column, value)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> PyResult<String> {
        self.model
            .get_formatted_cell_value(sheet, row, column)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn to_bytes(&self) -> PyResult<Vec<u8>> {
        let bytes = self.model.to_bytes();
        Ok(bytes)
    }

    // Properties access
    pub fn get_worksheets_properties(&self) -> PyResult<String> {
        serde_json::to_string(&self.model.get_worksheets_properties())
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Selection and navigation
    pub fn get_selected_sheet(&self) -> u32 {
        self.model.get_selected_sheet()
    }

    pub fn get_selected_cell(&self) -> Vec<i32> {
        let (sheet, row, column) = self.model.get_selected_cell();
        vec![sheet as i32, row, column]
    }

    pub fn get_selected_view(&self) -> PyResult<String> {
        serde_json::to_string(&self.model.get_selected_view())
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn set_selected_sheet(&mut self, sheet: u32) -> PyResult<()> {
        self.model.set_selected_sheet(sheet).map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn set_selected_cell(&mut self, row: i32, column: i32) -> PyResult<()> {
        self.model
            .set_selected_cell(row, column)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn set_selected_range(&mut self, start_row: i32, start_column: i32, end_row: i32, end_column: i32) -> PyResult<()> {
        self.model
            .set_selected_range(start_row, start_column, end_row, end_column)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn set_top_left_visible_cell(&mut self, top_row: i32, top_column: i32) -> PyResult<()> {
        self.model
            .set_top_left_visible_cell(top_row, top_column)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Grid options
    pub fn set_show_grid_lines(&mut self, sheet: u32, show_grid_lines: bool) -> PyResult<()> {
        self.model
            .set_show_grid_lines(sheet, show_grid_lines)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn get_show_grid_lines(&mut self, sheet: u32) -> PyResult<bool> {
        self.model.get_show_grid_lines(sheet).map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Auto-fill
    pub fn auto_fill_rows(&mut self, source_area: &PyArea, to_row: i32) -> PyResult<()> {
        let area = source_area.clone().into();
        self.model
            .auto_fill_rows(&area, to_row)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn auto_fill_columns(&mut self, source_area: &PyArea, to_column: i32) -> PyResult<()> {
        let area = source_area.clone().into();
        self.model
            .auto_fill_columns(&area, to_column)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Navigation methods
    pub fn on_arrow_right(&mut self) -> PyResult<()> {
        self.model.on_arrow_right().map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn on_arrow_left(&mut self) -> PyResult<()> {
        self.model.on_arrow_left().map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn on_arrow_up(&mut self) -> PyResult<()> {
        self.model.on_arrow_up().map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn on_arrow_down(&mut self) -> PyResult<()> {
        self.model.on_arrow_down().map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn on_page_down(&mut self) -> PyResult<()> {
        self.model.on_page_down().map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn on_page_up(&mut self) -> PyResult<()> {
        self.model.on_page_up().map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Window sizing
    pub fn set_window_width(&mut self, window_width: f64) {
        self.model.set_window_width(window_width);
    }

    pub fn set_window_height(&mut self, window_height: f64) {
        self.model.set_window_height(window_height);
    }

    pub fn get_scroll_x(&self) -> PyResult<f64> {
        self.model.get_scroll_x().map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn get_scroll_y(&self) -> PyResult<f64> {
        self.model.get_scroll_y().map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Selection expansion
    pub fn on_expand_selected_range(&mut self, key: &str) -> PyResult<()> {
        self.model
            .on_expand_selected_range(key)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn on_area_selecting(&mut self, target_row: i32, target_column: i32) -> PyResult<()> {
        self.model
            .on_area_selecting(target_row, target_column)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Borders - using JSON string approach like other bindings
    pub fn set_area_with_border(&mut self, area: &PyArea, border_area_json: &str) -> PyResult<()> {
        let range = area.clone().into();
        let border: xlsx::base::BorderArea = serde_json::from_str(border_area_json)
            .map_err(|e| WorkbookError::new_err(format!("Invalid border area JSON: {}", e)))?;
        self.model
            .set_area_with_border(&range, &border)
            .map_err(|e| WorkbookError::new_err(e.to_string()))?;
        Ok(())
    }

    // Model metadata
    pub fn get_name(&self) -> String {
        self.model.get_name()
    }

    pub fn set_name(&mut self, name: &str) {
        self.model.set_name(name);
    }

    // Clipboard operations
    pub fn copy_to_clipboard(&self) -> PyResult<String> {
        let data = self
            .model
            .copy_to_clipboard()
            .map_err(|e| WorkbookError::new_err(e.to_string()))?;
        serde_json::to_string(&data).map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn paste_from_clipboard(
        &mut self,
        source_sheet: u32,
        source_range: (i32, i32, i32, i32),
        clipboard: &str,
        is_cut: bool,
    ) -> PyResult<()> {
        let clipboard_data: xlsx::base::ClipboardData = serde_json::from_str(clipboard)
            .map_err(|e| WorkbookError::new_err(e.to_string()))?;
        self.model
            .paste_from_clipboard(source_sheet, source_range, &clipboard_data, is_cut)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn paste_csv_text(&mut self, area: &PyArea, csv: &str) -> PyResult<()> {
        let range = area.clone().into();
        self.model
            .paste_csv_string(&range, csv)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Data inspection functions (from WASM)
    pub fn get_rows_with_data(&self, sheet: u32, column: i32) -> PyResult<Vec<i32>> {
        let sheet_data = &self
            .model
            .get_model()
            .workbook
            .worksheet(sheet)
            .map_err(|e| WorkbookError::new_err(e.to_string()))?
            .sheet_data;
        Ok(sheet_data
            .iter()
            .filter(|(_, data)| data.contains_key(&column))
            .map(|(row, _)| *row)
            .collect())
    }

    pub fn get_columns_with_data(&self, sheet: u32, row: i32) -> PyResult<Vec<i32>> {
        Ok(self
            .model
            .get_model()
            .workbook
            .worksheet(sheet)
            .map_err(|e| WorkbookError::new_err(e.to_string()))?
            .sheet_data
            .get(&row)
            .map(|row_data| row_data.keys().copied().collect())
            .unwrap_or_default())
    }

    pub fn get_last_non_empty_in_row_before_column(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> PyResult<Option<i32>> {
        self.model
            .get_last_non_empty_in_row_before_column(sheet, row, column)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn get_first_non_empty_in_row_after_column(
        &self,
        sheet: u32,
        row: i32,
        column: i32,
    ) -> PyResult<Option<i32>> {
        self.model
            .get_first_non_empty_in_row_after_column(sheet, row, column)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Navigation function
    pub fn on_navigate_to_edge_in_direction(&mut self, direction: &str) -> PyResult<()> {
        let nav_direction = match direction {
            "ArrowUp" => xlsx::base::worksheet::NavigationDirection::Up,
            "ArrowDown" => xlsx::base::worksheet::NavigationDirection::Down,
            "ArrowLeft" => xlsx::base::worksheet::NavigationDirection::Left,
            "ArrowRight" => xlsx::base::worksheet::NavigationDirection::Right,
            _ => return Err(WorkbookError::new_err("Invalid direction")),
        };
        self.model
            .on_navigate_to_edge_in_direction(nav_direction)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Action functions with different names (to match WASM naming)
    pub fn move_column_action(&mut self, sheet: u32, column: i32, delta: i32) -> PyResult<()> {
        self.model
            .move_column_action(sheet, column, delta)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn move_row_action(&mut self, sheet: u32, row: i32, delta: i32) -> PyResult<()> {
        self.model
            .move_row_action(sheet, row, delta)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }
}

/// This is a model implementing the 'raw' API
#[pyclass]
pub struct PyModel {
    model: Model,
}

#[pymethods]
impl PyModel {
    /// Saves the model to an xlsx file
    pub fn save_to_xlsx(&self, file: &str) -> PyResult<()> {
        save_to_xlsx(&self.model, file).map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    /// Saves the model to file in the internal binary ic format
    pub fn save_to_icalc(&self, file: &str) -> PyResult<()> {
        save_to_icalc(&self.model, file).map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    /// To bytes
    pub fn to_bytes(&self) -> PyResult<Vec<u8>> {
        let bytes = self.model.to_bytes();
        Ok(bytes)
    }

    /// Evaluates the workbook
    pub fn evaluate(&mut self) {
        self.model.evaluate()
    }

    // Set values

    /// Set an input
    pub fn set_user_input(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        value: &str,
    ) -> PyResult<()> {
        self.model
            .set_user_input(sheet, row, column, value.to_string())
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn clear_cell_contents(&mut self, sheet: u32, row: i32, column: i32) -> PyResult<()> {
        self.model
            .cell_clear_contents(sheet, row, column)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Get values

    /// Get raw value
    pub fn get_cell_content(&self, sheet: u32, row: i32, column: i32) -> PyResult<String> {
        self.model
            .get_cell_content(sheet, row, column)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    /// Get cell type
    pub fn get_cell_type(&self, sheet: u32, row: i32, column: i32) -> PyResult<PyCellType> {
        self.model
            .get_cell_type(sheet, row, column)
            .map(|cell_type| cell_type.into())
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    /// Get formatted value
    pub fn get_formatted_cell_value(&self, sheet: u32, row: i32, column: i32) -> PyResult<String> {
        self.model
            .get_formatted_cell_value(sheet, row, column)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Set styles
    pub fn set_cell_style(
        &mut self,
        sheet: u32,
        row: i32,
        column: i32,
        py_style: &PyStyle,
    ) -> PyResult<()> {
        let style: Style = py_style.into();
        self.model
            .set_cell_style(sheet, row, column, &style)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Get styles
    pub fn get_cell_style(&self, sheet: u32, row: i32, column: i32) -> PyResult<PyStyle> {
        let style = self
            .model
            .get_style_for_cell(sheet, row, column)
            .map_err(|e| WorkbookError::new_err(e.to_string()))?;
        Ok(style.into())
    }

    // column widths, row heights
    // insert/delete rows/columns

    pub fn insert_rows(&mut self, sheet: u32, row: i32, row_count: i32) -> PyResult<()> {
        self.model
            .insert_rows(sheet, row, row_count)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn insert_columns(&mut self, sheet: u32, column: i32, column_count: i32) -> PyResult<()> {
        self.model
            .insert_columns(sheet, column, column_count)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn delete_rows(&mut self, sheet: u32, row: i32, row_count: i32) -> PyResult<()> {
        self.model
            .delete_rows(sheet, row, row_count)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn delete_columns(&mut self, sheet: u32, column: i32, column_count: i32) -> PyResult<()> {
        self.model
            .delete_columns(sheet, column, column_count)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn get_column_width(&self, sheet: u32, column: i32) -> PyResult<f64> {
        self.model
            .get_column_width(sheet, column)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn get_row_height(&self, sheet: u32, row: i32) -> PyResult<f64> {
        self.model
            .get_row_height(sheet, row)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn set_column_width(&mut self, sheet: u32, column: i32, width: f64) -> PyResult<()> {
        self.model
            .set_column_width(sheet, column, width)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn set_row_height(&mut self, sheet: u32, row: i32, height: f64) -> PyResult<()> {
        self.model
            .set_row_height(sheet, row, height)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // frozen rows/columns

    pub fn get_frozen_columns_count(&self, sheet: u32) -> PyResult<i32> {
        self.model
            .get_frozen_columns_count(sheet)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn get_frozen_rows_count(&self, sheet: u32) -> PyResult<i32> {
        self.model
            .get_frozen_rows_count(sheet)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn set_frozen_columns_count(&mut self, sheet: u32, column_count: i32) -> PyResult<()> {
        self.model
            .set_frozen_columns(sheet, column_count)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn set_frozen_rows_count(&mut self, sheet: u32, row_count: i32) -> PyResult<()> {
        self.model
            .set_frozen_rows(sheet, row_count)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Manipulate sheets (add/remove/rename/change color)
    pub fn get_worksheets_properties(&self) -> PyResult<Vec<PySheetProperty>> {
        Ok(self
            .model
            .get_worksheets_properties()
            .into_iter()
            .map(|s| PySheetProperty {
                name: s.name,
                state: s.state,
                sheet_id: s.sheet_id,
                color: s.color,
            })
            .collect())
    }

    pub fn set_sheet_color(&mut self, sheet: u32, color: &str) -> PyResult<()> {
        self.model
            .set_sheet_color(sheet, color)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn add_sheet(&mut self, sheet_name: &str) -> PyResult<()> {
        self.model
            .add_sheet(sheet_name)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn new_sheet(&mut self) {
        self.model.new_sheet();
    }

    pub fn delete_sheet(&mut self, sheet: u32) -> PyResult<()> {
        self.model
            .delete_sheet(sheet)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn rename_sheet(&mut self, sheet: u32, new_name: &str) -> PyResult<()> {
        self.model
            .rename_sheet_by_index(sheet, new_name)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Move rows and columns
    pub fn move_row(&mut self, sheet: u32, row: i32, delta: i32) -> PyResult<()> {
        self.model
            .move_row_action(sheet, row, delta)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn move_column(&mut self, sheet: u32, column: i32, delta: i32) -> PyResult<()> {
        self.model
            .move_column_action(sheet, column, delta)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    // Defined names helpers - PyModel uses workbook directly
    pub fn get_defined_name_list(&self) -> Vec<PyDefinedName> {
        self.model
            .workbook
            .get_defined_names_with_scope()
            .iter()
            .map(|(name, scope, formula)| PyDefinedName { 
                name: name.to_owned(), 
                scope: *scope, 
                formula: formula.to_owned() 
            })
            .collect()
    }

    pub fn new_defined_name(
        &mut self,
        name: &str,
        scope: Option<u32>,
        formula: &str,
    ) -> PyResult<()> {
        self.model
            .new_defined_name(name, scope, formula)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn update_defined_name(
        &mut self,
        name: &str,
        scope: Option<u32>,
        new_name: &str,
        new_scope: Option<u32>,
        new_formula: &str,
    ) -> PyResult<()> {
        self.model
            .update_defined_name(name, scope, new_name, new_scope, new_formula)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    pub fn delete_defined_name(&mut self, name: &str, scope: Option<u32>) -> PyResult<()> {
        self.model
            .delete_defined_name(name, scope)
            .map_err(|e| WorkbookError::new_err(e.to_string()))
    }

    #[allow(clippy::panic)]
    pub fn test_panic(&self) -> PyResult<()> {
        panic!("This function panics for testing panic handling");
    }
}

// Create methods

/// Loads a function from an xlsx file
#[pyfunction]
pub fn load_from_xlsx(file_path: &str, locale: &str, tz: &str) -> PyResult<PyModel> {
    let model = import::load_from_xlsx(file_path, locale, tz)
        .map_err(|e| WorkbookError::new_err(e.to_string()))?;
    Ok(PyModel { model })
}

/// Loads a function from icalc binary representation
#[pyfunction]
pub fn load_from_icalc(file_name: &str) -> PyResult<PyModel> {
    let model =
        import::load_from_icalc(file_name).map_err(|e| WorkbookError::new_err(e.to_string()))?;
    Ok(PyModel { model })
}

/// Loads a model from bytes
/// This function expects the bytes to be in the internal binary ic format
/// which is the same format used by the `save_to_icalc` function.
#[pyfunction]
pub fn load_from_bytes(bytes: &[u8]) -> PyResult<PyModel> {
    let workbook: Workbook =
        bitcode::decode(bytes).map_err(|e| WorkbookError::new_err(e.to_string()))?;
    let model =
        Model::from_workbook(workbook).map_err(|e| WorkbookError::new_err(e.to_string()))?;
    Ok(PyModel { model })
}

/// Creates an empty model in the raw API
#[pyfunction]
pub fn create(name: &str, locale: &str, tz: &str) -> PyResult<PyModel> {
    let model =
        Model::new_empty(name, locale, tz).map_err(|e| WorkbookError::new_err(e.to_string()))?;
    Ok(PyModel { model })
}

/// Creates a model with the user model API
#[pyfunction]
pub fn create_user_model(name: &str, locale: &str, tz: &str) -> PyResult<PyUserModel> {
    let model = UserModel::new_empty(name, locale, tz)
        .map_err(|e| WorkbookError::new_err(e.to_string()))?;
    Ok(PyUserModel { model })
}

/// Creates a user model from an Excel file
#[pyfunction]
pub fn create_user_model_from_xlsx(
    file_path: &str,
    locale: &str,
    tz: &str,
) -> PyResult<PyUserModel> {
    let model = import::load_from_xlsx(file_path, locale, tz)
        .map_err(|e| WorkbookError::new_err(e.to_string()))?;
    let model = UserModel::from_model(model);
    Ok(PyUserModel { model })
}

/// Creates a user model from an icalc file
#[pyfunction]
pub fn create_user_model_from_icalc(file_name: &str) -> PyResult<PyUserModel> {
    let model =
        import::load_from_icalc(file_name).map_err(|e| WorkbookError::new_err(e.to_string()))?;
    let model = UserModel::from_model(model);
    Ok(PyUserModel { model })
}

/// Creates a user model from bytes
/// This function expects the bytes to be in the internal binary ic format
/// which is the same format used by the `save_to_icalc` function.
#[pyfunction]
pub fn create_user_model_from_bytes(bytes: &[u8]) -> PyResult<PyUserModel> {
    let workbook: Workbook =
        bitcode::decode(bytes).map_err(|e| WorkbookError::new_err(e.to_string()))?;
    let model =
        Model::from_workbook(workbook).map_err(|e| WorkbookError::new_err(e.to_string()))?;
    let user_model = UserModel::from_model(model);
    Ok(PyUserModel { model: user_model })
}

#[pyfunction]
pub fn get_tokens(formula: &str) -> PyResult<String> {
    let tokens = xlsx::base::expressions::lexer::util::get_tokens(formula);
    serde_json::to_string(&tokens).map_err(|e| WorkbookError::new_err(e.to_string()))
}

#[pyfunction]
pub fn column_name_from_number(column: i32) -> PyResult<String> {
    match xlsx::base::expressions::utils::number_to_column(column) {
        Some(c) => Ok(c),
        None => Err(WorkbookError::new_err("Invalid column number")),
    }
}

#[pyfunction]
#[allow(clippy::panic)]
pub fn test_panic() {
    panic!("This function panics for testing panic handling");
}

#[pymodule]
fn ironcalc(m: &Bound<'_, PyModule>) -> PyResult<()> {
    // Add the package version to the module
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;

    // Add the functions to the module using the `?` operator
    m.add_function(wrap_pyfunction!(create, m)?)?;
    m.add_function(wrap_pyfunction!(load_from_xlsx, m)?)?;
    m.add_function(wrap_pyfunction!(load_from_icalc, m)?)?;
    m.add_function(wrap_pyfunction!(load_from_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(get_tokens, m)?)?;
    m.add_function(wrap_pyfunction!(column_name_from_number, m)?)?;
    m.add_function(wrap_pyfunction!(test_panic, m)?)?;

    // User model functions
    m.add_function(wrap_pyfunction!(create_user_model, m)?)?;
    m.add_function(wrap_pyfunction!(create_user_model_from_bytes, m)?)?;
    m.add_function(wrap_pyfunction!(create_user_model_from_xlsx, m)?)?;
    m.add_function(wrap_pyfunction!(create_user_model_from_icalc, m)?)?;

    // Add classes
    m.add_class::<PyModel>()?;
    m.add_class::<PyUserModel>()?;
    m.add_class::<types::PyArea>()?;

    m.add_class::<types::PyDefinedName>()?;
    m.add_class::<types::PyStyle>()?;
    m.add_class::<types::PyCellType>()?;
    m.add_class::<types::PySheetProperty>()?;

    Ok(())
}
