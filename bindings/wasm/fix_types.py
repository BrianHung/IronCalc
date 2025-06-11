# Regrettably at the time of writing there is not a perfect way to
# generate the TypeScript types from Rust so we basically fix them manually
# Hopefully this will suffice for our needs and one day will be automatic

header = r"""
/* tslint:disable */
/* eslint-disable */
""".strip()

get_tokens_str = r"""
* @returns {any}
*/
export function getTokens(formula: string): any;
""".strip()

get_tokens_str_types = r"""
* @returns {MarkedToken[]}
*/
export function getTokens(formula: string): MarkedToken[];
""".strip()

update_style_str = r"""
/**
* @param {any} range
* @param {string} style_path
* @param {string} value
*/
  updateRangeStyle(range: any, style_path: string, value: string): void;
""".strip()

update_style_str_types = r"""
/**
* @param {Area} range
* @param {string} style_path
* @param {string} value
*/
  updateRangeStyle(range: Area, style_path: string, value: string): void;
""".strip()

properties = r"""
/**
* @returns {any}
*/
  getWorksheetsProperties(): any;
""".strip()

properties_types = r"""
/**
* @returns {WorksheetProperties[]}
*/
  getWorksheetsProperties(): WorksheetProperties[];
""".strip()

style = r"""
* @returns {any}
*/
  getCellStyle(sheet: number, row: number, column: number): any;
""".strip()

style_types = r"""
* @returns {CellStyle}
*/
  getCellStyle(sheet: number, row: number, column: number): CellStyle;
""".strip()

view = r"""
* @returns {any}
*/
  getSelectedView(): any;
""".strip()

view_types = r"""
* @returns {CellStyle}
*/
  getSelectedView(): SelectedView;
""".strip()

autofill_rows = r"""
/**
* @param {any} source_area
* @param {number} to_row
*/
  autoFillRows(source_area: any, to_row: number): void;
"""

autofill_rows_types = r"""
/**
* @param {Area} source_area
* @param {number} to_row
*/
  autoFillRows(source_area: Area, to_row: number): void;
"""

autofill_columns = r"""
/**
* @param {any} source_area
* @param {number} to_column
*/
  autoFillColumns(source_area: any, to_column: number): void;
"""

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

def check_types(text: str) -> None:
    """Ensure generated definitions don't contain any stray 'any' types."""
    if text.find("any") != -1:
        print("There are 'unfixed' types. Please check.")
        exit(1)


def fix_types(ts_file: str) -> None:
    """Validate generated TypeScript definitions."""
    with open(ts_file) as f:
        text = f.read()
    check_types(text)


def append_js(js_file: str) -> None:
    """Prepend the generated TypeScript enums JS to the wasm bundle."""

    with open("types.js") as f:
        text_js = f.read()
    with open(js_file) as f:
        text = f.read()

    with open(js_file, "wb") as f:
        f.write(bytes("{}\n{}".format(text_js, text), "utf8"))
    


if __name__ == "__main__":
    ts_file = "pkg/wasm.d.ts"
    fix_types(ts_file)

    js_file = "pkg/wasm.js"
    append_js(js_file)
    

    
