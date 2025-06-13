export interface Area {
  sheet: number;
  row: number;
  column: number;
  width: number;
  height: number;
}

export enum BorderType {
  All = "All",
  Inner = "Inner",
  Outer = "Outer",
  Top = "Top",
  Right = "Right",
  Bottom = "Bottom",
  Left = "Left",
  CenterH = "CenterH",
  CenterV = "CenterV",
  None = "None",
}

export interface BorderArea {
  item: BorderItem;
  type: BorderType;
}

type ErrorType =
  | "REF"
  | "NAME"
  | "VALUE"
  | "DIV"
  | "NA"
  | "NUM"
  | "ERROR"
  | "NIMPL"
  | "SPILL"
  | "CALC"
  | "CIRC";

type OpCompareType =
  | "LessThan"
  | "GreaterThan"
  | "Equal"
  | "LessOrEqualThan"
  | "GreaterOrEqualThan"
  | "NonEqual";

type OpSumType = "Add" | "Minus";

type OpProductType = "Times" | "Divide";

interface ReferenceType {
  sheet: string | null;
  row: number;
  column: number;
  absolute_column: boolean;
  absolute_row: boolean;
}

interface ParsedReferenceType {
  column: number;
  row: number;
  absolute_column: boolean;
  absolute_row: boolean;
}

interface Reference {
  Reference: ReferenceType;
}

interface Range {
  Range: {
    sheet: string | null;
    left: ParsedReferenceType;
    right: ParsedReferenceType;
  };
}

export type TokenType =
  | "Illegal"
  | "Eof"
  | { Ident: string }
  | { String: string }
  | { Boolean: boolean }
  | { Number: number }
  | { ERROR: ErrorType }
  | { COMPARE: OpCompareType }
  | { SUM: OpSumType }
  | { PRODUCT: OpProductType }
  | "POWER"
  | "LPAREN"
  | "RPAREN"
  | "COLON"
  | "SEMICOLON"
  | "LBRACKET"
  | "RBRACKET"
  | "LBRACE"
  | "RBRACE"
  | "COMMA"
  | "BANG"
  | "PERCENT"
  | "AND"
  | Reference
  | Range;

export interface MarkedToken {
  token: TokenType;
  start: number;
  end: number;
}

export interface WorksheetProperties {
  name: string;
  color: string;
  sheet_id: number;
  state: string;
}

export interface NewSheetResult {
  name: string;
  sheet_index: number;
}

interface CellStyleFill {
  pattern_type: string;
  fg_color?: string;
  bg_color?: string;
}

interface CellStyleFont {
  u: boolean;
  b: boolean;
  i: boolean;
  strike: boolean;
  sz: number;
  color: string;
  name: string;
  family: number;
  scheme: string;
}

export interface BorderOptions {
  color: string;
  style: BorderStyle;
  border: BorderType;
}

export enum BorderStyle {
  Thin = "thin",
  Medium = "medium",
  Thick = "thick",
  Double = "double",
  Dotted = "dotted",
  SlantDashDot = "slantdashdot",
  MediumDashed = "mediumdashed",
  MediumDashDotDot = "mediumdashdotdot",
  MediumDashDot = "mediumdashdot",
}

interface BorderItem {
  style: string;
  color: string;
}

interface CellStyleBorder {
  diagonal_up?: boolean;
  diagonal_down?: boolean;
  left: BorderItem;
  right: BorderItem;
  top: BorderItem;
  bottom: BorderItem;
  diagonal: BorderItem;
}

export type VerticalAlignment =
  | "bottom"
  | "center"
  | "distributed"
  | "justify"
  | "top";

export type HorizontalAlignment =
  | "left"
  | "center"
  | "right"
  | "general"
  | "centerContinuous"
  | "distributed"
  | "fill"
  | "justify";

interface Alignment {
  horizontal: HorizontalAlignment;
  vertical: VerticalAlignment;
  wrap_text: boolean;
}

export interface CellStyle {
  read_only: boolean;
  quote_prefix: boolean;
  fill: CellStyleFill;
  font: CellStyleFont;
  border: CellStyleBorder;
  num_fmt: string;
  alignment?: Alignment;
}

export interface SelectedView {
  sheet: number;
  row: number;
  column: number;
  range: [number, number, number, number];
  top_row: number;
  left_column: number;
}

export interface WorksheetView {
  row: number;
  column: number;
  range: [number, number, number, number];
  top_row: number;
  left_column: number;
}

// type ClipboardData = {
//   [row: number]: {
//       [column: number]: ClipboardCell;
//   };
// };

// type ClipboardData = Record<string, Record <string, ClipboardCell>>;
type ClipboardData = Map<number, Map <number, ClipboardCell>>;

export interface ClipboardCell {
  text: string;
  style: CellStyle;
}

export interface Clipboard {
  csv: string;
  data: ClipboardData;
  range: [number, number, number, number];
}

export interface DefinedName {
  name: string;
  scope?: number;
  formula: string;
}

// Types for Diffs and QueueDiffs used in getRecentDiffs
export interface CellReference {
  sheet: number;
  row: number;
  column: number;
}

export interface Cell {
  type: string; // e.g., "NumberCell", "SharedString", "BooleanCell"
  v?: number | boolean | string; // value, if applicable
  s: number; // style index
  f?: number; // formula index, if applicable
  ei?: string; // error type, if applicable
  o?: string; // origin, if applicable
  m?: string; // error message, if applicable
}

export interface Style {
  read_only?: boolean;
  quote_prefix: boolean;
  fill: {
    pattern_type: string;
    fg_color?: string;
    bg_color?: string;
  };
  font: {
    u: boolean;
    b: boolean;
    i: boolean;
    strike: boolean;
    sz: number;
    color?: string;
    name: string;
    family: number;
    scheme: string;
  };
  border: {
    diagonal_up?: boolean;
    diagonal_down?: boolean;
    left?: { style: string; color?: string };
    right?: { style: string; color?: string };
    top?: { style: string; color?: string };
    bottom?: { style: string; color?: string };
    diagonal?: { style: string; color?: string };
  };
  num_fmt: string;
  alignment?: {
    horizontal: HorizontalAlignment;
    vertical: VerticalAlignment;
    wrap_text: boolean;
  };
}

export interface RowData {
  row?: { r: number; height: number; custom_format: boolean; custom_height: boolean; s: number; hidden: boolean };
  data: Record<number, Cell>;
}

export interface ColumnData {
  column?: { min: number; max: number; width: number; custom_width: boolean; style?: number };
  data: Record<number, Cell>;
}

export interface Worksheet {
  dimension: string;
  cols: Array<{ min: number; max: number; width: number; custom_width: boolean; style?: number }>;
  rows: Array<{ r: number; height: number; custom_format: boolean; custom_height: boolean; s: number; hidden: boolean }>;
  name: string;
  sheet_data: Record<number, Record<number, Cell>>;
  shared_formulas: string[];
  sheet_id: number;
  state: string; // e.g., "Visible", "Hidden", "VeryHidden"
  color?: string;
  merge_cells: string[];
  comments: Array<{ text: string; author_name: string; author_id?: string; cell_ref: string }>;
  frozen_rows: number;
  frozen_columns: number;
  views: Record<number, WorksheetView>;
  show_grid_lines: boolean;
}

// Individual Diff type interfaces for better developer experience
export interface SetCellValueDiff {
  type: "setCellValue";
  sheet: number;
  row: number;
  column: number;
  new_value: string;
  old_value?: Cell | null;
}

export interface CellClearContentsDiff {
  type: "cellClearContents";
  sheet: number;
  row: number;
  column: number;
  old_value?: Cell | null;
}

export interface CellClearAllDiff {
  type: "cellClearAll";
  sheet: number;
  row: number;
  column: number;
  old_value?: Cell | null;
  old_style: Style;
}

export interface CellClearFormattingDiff {
  type: "cellClearFormatting";
  sheet: number;
  row: number;
  column: number;
  old_style?: Style | null;
}

export interface SetCellStyleDiff {
  type: "setCellStyle";
  sheet: number;
  row: number;
  column: number;
  old_value?: Style | null;
  new_value: Style;
}

export interface SetColumnWidthDiff {
  type: "setColumnWidth";
  sheet: number;
  column: number;
  new_value: number;
  old_value: number;
}

export interface SetRowHeightDiff {
  type: "setRowHeight";
  sheet: number;
  row: number;
  new_value: number;
  old_value: number;
}

export interface SetColumnStyleDiff {
  type: "setColumnStyle";
  sheet: number;
  column: number;
  old_value?: Style | null;
  new_value: Style;
}

export interface SetRowStyleDiff {
  type: "setRowStyle";
  sheet: number;
  row: number;
  old_value?: Style | null;
  new_value: Style;
}

export interface DeleteColumnStyleDiff {
  type: "deleteColumnStyle";
  sheet: number;
  column: number;
  old_value?: Style | null;
}

export interface DeleteRowStyleDiff {
  type: "deleteRowStyle";
  sheet: number;
  row: number;
  old_value?: Style | null;
}

export interface InsertRowDiff {
  type: "insertRow";
  sheet: number;
  row: number;
}

export interface DeleteRowDiff {
  type: "deleteRow";
  sheet: number;
  row: number;
  old_data: RowData;
}

export interface InsertColumnDiff {
  type: "insertColumn";
  sheet: number;
  column: number;
}

export interface DeleteColumnDiff {
  type: "deleteColumn";
  sheet: number;
  column: number;
  old_data: ColumnData;
}

export interface DeleteSheetDiff {
  type: "deleteSheet";
  sheet: number;
  old_data: Worksheet;
}

export interface SetFrozenRowsCountDiff {
  type: "setFrozenRowsCount";
  sheet: number;
  new_value: number;
  old_value: number;
}

export interface SetFrozenColumnsCountDiff {
  type: "setFrozenColumnsCount";
  sheet: number;
  new_value: number;
  old_value: number;
}

export interface NewSheetDiff {
  type: "newSheet";
  index: number;
  name: string;
}

export interface RenameSheetDiff {
  type: "renameSheet";
  index: number;
  old_value: string;
  new_value: string;
}

export interface SetSheetColorDiff {
  type: "setSheetColor";
  index: number;
  old_value: string;
  new_value: string;
}

export interface SetSheetStateDiff {
  type: "setSheetState";
  index: number;
  old_value: string;
  new_value: string;
}

export interface SetShowGridLinesDiff {
  type: "setShowGridLines";
  sheet: number;
  old_value: boolean;
  new_value: boolean;
}

export interface CreateDefinedNameDiff {
  type: "createDefinedName";
  name: string;
  scope?: number;
  value: string;
}

export interface DeleteDefinedNameDiff {
  type: "deleteDefinedName";
  name: string;
  scope?: number;
  old_value: string;
}

export interface UpdateDefinedNameDiff {
  type: "updateDefinedName";
  name: string;
  scope?: number;
  old_formula: string;
  new_name: string;
  new_scope?: number;
  new_formula: string;
}

export type Diff = 
  | SetCellValueDiff
  | CellClearContentsDiff
  | CellClearAllDiff
  | CellClearFormattingDiff
  | SetCellStyleDiff
  | SetColumnWidthDiff
  | SetRowHeightDiff
  | SetColumnStyleDiff
  | SetRowStyleDiff
  | DeleteColumnStyleDiff
  | DeleteRowStyleDiff
  | InsertRowDiff
  | DeleteRowDiff
  | InsertColumnDiff
  | DeleteColumnDiff
  | DeleteSheetDiff
  | SetFrozenRowsCountDiff
  | SetFrozenColumnsCountDiff
  | NewSheetDiff
  | RenameSheetDiff
  | SetSheetColorDiff
  | SetSheetStateDiff
  | SetShowGridLinesDiff
  | CreateDefinedNameDiff
  | DeleteDefinedNameDiff
  | UpdateDefinedNameDiff;

// Interface for QueueDiffs
export interface QueueDiffs {
  type: "undo" | "redo";
  list: Diff[];
}
export interface WorksheetDimension {
  min_row: number;
  max_row: number;
  min_column: number;
  max_column: number;
}
