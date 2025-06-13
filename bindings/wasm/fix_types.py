# Regrettably at the time of writing there is not a perfect way to
# generate the TypeScript types from Rust so we basically fix them manually
# Hopefully this will suffice for our needs and one day will be automatic
# Updated patterns for wasm-bindgen >=0.2.94 (two-space indent, no @returns docs)

header = r"""
/* tslint:disable */
/* eslint-disable */
""".strip()


get_tokens_str = "export function getTokens(formula: string): any;"
get_tokens_str_types = "export function getTokens(formula: string): MarkedToken[];"

update_style_str = "  updateRangeStyle(range: any, style_path: string, value: string): void;"
update_style_str_types = "  updateRangeStyle(range: Area, style_path: string, value: string): void;"

properties = "  getWorksheetsProperties(): any;"
properties_types = "  getWorksheetsProperties(): WorksheetProperties[];"

style = "  getCellStyle(sheet: number, row: number, column: number): any;"
style_types = "  getCellStyle(sheet: number, row: number, column: number): CellStyle;"

view = "  getSelectedView(): any;"
view_types = "  getSelectedView(): SelectedView;"

autofill_rows = "  autoFillRows(source_area: any, to_row: number): void;"
autofill_rows_types = "  autoFillRows(source_area: Area, to_row: number): void;"

autofill_columns = "  autoFillColumns(source_area: any, to_column: number): void;"
autofill_columns_types = "  autoFillColumns(source_area: Area, to_column: number): void;"

set_cell_style = "  onPasteStyles(styles: any): void;"
set_cell_style_types = "  onPasteStyles(styles: CellStyle[][]): void;"

set_area_border = "  setAreaWithBorder(area: any, border_area: any): void;"
set_area_border_types = "  setAreaWithBorder(area: Area, border_area: BorderArea): void;"

paste_csv_string = "  pasteCsvText(area: any, csv: string): void;"
paste_csv_string_types = "  pasteCsvText(area: Area, csv: string): void;"

clipboard = "  copyToClipboard(): any;"
clipboard_types = "  copyToClipboard(): Clipboard;"

paste_from_clipboard = "  pasteFromClipboard(source_sheet: number, source_range: any, clipboard: any, is_cut: boolean): void;"
paste_from_clipboard_types = "  pasteFromClipboard(source_sheet: number, source_range: [number, number, number, number], clipboard: ClipboardData, is_cut: boolean): void;"

autofill_columns_types = r"""
/**
* @param {Area} source_area
* @param {number} to_column
*/
  autoFillColumns(source_area: Area, to_column: number): void;
"""

set_cell_style = r"""
/**
* @param {any} styles
*/
  onPasteStyles(styles: any): void;
"""

set_cell_style_types = r"""
/**
* @param {CellStyle[][]} styles
*/
  onPasteStyles(styles: CellStyle[][]): void;
"""

set_area_border = r"""
/**
* @param {any} area
* @param {any} border_area
*/
  setAreaWithBorder(area: any, border_area: any): void;
"""

set_area_border_types = r"""
/**
* @param {Area} area
* @param {BorderArea} border_area
*/
  setAreaWithBorder(area: Area, border_area: BorderArea): void;
"""

paste_csv_string = r"""
/**
* @param {any} area
* @param {string} csv
*/
  pasteCsvText(area: any, csv: string): void;
"""

paste_csv_string_types = r"""
/**
* @param {Area} area
* @param {string} csv
*/
  pasteCsvText(area: Area, csv: string): void;
"""

clipboard = r"""
/**
* @returns {any}
*/
  copyToClipboard(): any;
"""

clipboard_types = r"""
/**
* @returns {Clipboard}
*/
  copyToClipboard(): Clipboard;
"""

get_changed_cells = r"""
/**
* @returns {any}
*/
  getChangedCells(): any;
"""

get_changed_cells_types = r"""
/**
* @returns {ChangedCell[]}
*/
  getChangedCells(): ChangedCell[];
"""

paste_from_clipboard = r"""
/**
* @param {number} source_sheet
* @param {any} source_range
* @param {any} clipboard
* @param {boolean} is_cut
*/
  pasteFromClipboard(source_sheet: number, source_range: any, clipboard: any, is_cut: boolean): void;
"""

paste_from_clipboard_types = r"""
/**
* @param {number} source_sheet
* @param {[number, number, number, number]} source_range
* @param {ClipboardData} clipboard
* @param {boolean} is_cut
*/
  pasteFromClipboard(source_sheet: number, source_range: [number, number, number, number], clipboard: ClipboardData, is_cut: boolean): void;
"""

defined_name_list = r"""
/**
* @returns {any}
*/
  getDefinedNameList(): any;
"""

defined_name_list_types = r"""
/**
* @returns {DefinedName[]}
*/
  getDefinedNameList(): DefinedName[];
"""

get_recent_diffs = r"""
/**
* @returns {any}
*/
  getRecentDiffs(): any;
"""

get_recent_diffs_types = r"""
/**
* @returns {QueueDiffs[]}
*/
  getRecentDiffs(): QueueDiffs[];
"""

new_sheet = r"""
/**
* @returns {any}
*/
  newSheet(): any;
"""

new_sheet_types = r"""
/**
* @returns {NewSheetResult}
*/
  newSheet(): NewSheetResult;
"""

get_sheet_dimensions = r"""
/**
* @param {number} sheet
* @returns {any}
*/
  getSheetDimensions(sheet: number): any;
"""

get_sheet_dimensions_types = r"""
/**
* @param {number} sheet
* @returns {WorksheetDimension}
*/
  getSheetDimensions(sheet: number): WorksheetDimension;
"""

def fix_types(text):
    text = text.replace(get_tokens_str, get_tokens_str_types)
    text = text.replace(update_style_str, update_style_str_types)
    text = text.replace(properties, properties_types)
    text = text.replace(style, style_types)
    text = text.replace(view, view_types)
    text = text.replace(autofill_rows, autofill_rows_types)
    text = text.replace(autofill_columns, autofill_columns_types)
    text = text.replace(set_cell_style, set_cell_style_types)
    text = text.replace(set_area_border, set_area_border_types)
    text = text.replace(paste_csv_string, paste_csv_string_types)
    text = text.replace(clipboard, clipboard_types)
    text = text.replace(get_changed_cells, get_changed_cells_types)
    text = text.replace(paste_from_clipboard, paste_from_clipboard_types)
    text = text.replace(defined_name_list, defined_name_list_types)
    text = text.replace(get_recent_diffs, get_recent_diffs_types)
    text = text.replace(new_sheet, new_sheet_types)
    text = text.replace(get_sheet_dimensions, get_sheet_dimensions_types)
    with open("types.ts") as f:
        types_str = f.read()
        header_types = "{}\n\n{}".format(header, types_str)
    text = text.replace(header, header_types)
    for line in text.splitlines():
        stripped = line.lstrip()
        # Skip internal methods
        if stripped.startswith("readonly model_"):
            continue
        if stripped.find("any") != -1:
            print("There are 'unfixed' public types. Please check.")
            exit(1)
    return text
    


if __name__ == "__main__":
    dts_files = ["pkg/ironcalc.d.ts", "pkg/xlsx.d.ts"]
    for types_file in dts_files:
        with open(types_file) as f:
            text = f.read()
        text = fix_types(text)
        with open(types_file, "wb") as f:
            f.write(bytes(text, "utf8"))

    js_files = ["pkg/ironcalc.js", "pkg/xlsx.js"]
    with open("types.js") as f:
        text_js = f.read()

    for js_file in js_files:
        with open(js_file) as f:
            text = f.read()
        with open(js_file, "wb") as f:
            f.write(bytes("{}\n{}".format(text_js, text), "utf8"))
    

    
