import { styled } from "@mui/material/styles";
import { useEffect, useRef, useState } from "react";
import WorksheetCanvas from "./WorksheetCanvas/worksheetCanvas";
import {
  outlineBackgroundColor,
  outlineColor,
} from "./WorksheetCanvas/constants";
import usePointer from "./usePointer";
import { AreaType, WorkbookState } from "./workbookState";
import { Cell } from "./WorksheetCanvas/types";
import Editor from "./editor";
import EditorContext, { EditorState } from "./editor/editorContext";
import { getFormulaHTML } from "./editor/util";
import { Model } from "@ironcalc/wasm";

function Worksheet(props: {
  model: Model;
  workbookState: WorkbookState;
  refresh: () => void;
}) {
  const canvasElement = useRef<HTMLCanvasElement>(null);

  const worksheetElement = useRef<HTMLDivElement>(null);
  const scrollElement = useRef<HTMLDivElement>(null);
  // const rootElement = useRef<HTMLDivElement>(null);
  const spacerElement = useRef<HTMLDivElement>(null);
  const cellOutline = useRef<HTMLDivElement>(null);
  const areaOutline = useRef<HTMLDivElement>(null);
  const cellOutlineHandle = useRef<HTMLDivElement>(null);
  const extendToOutline = useRef<HTMLDivElement>(null);
  const columnResizeGuide = useRef<HTMLDivElement>(null);
  const rowResizeGuide = useRef<HTMLDivElement>(null);
  // const contextMenuAnchorElement = useRef<HTMLDivElement>(null);
  const columnHeaders = useRef<HTMLDivElement>(null);
  const worksheetCanvas = useRef<WorksheetCanvas | null>(null);

  const [isEditing, setEditing] = useState(false);
  const ignoreScrollEventRef = useRef(false);

  const [editorContext, setEditorContext] = useState<EditorState>({
    mode: "accept",
    insertRange: null,
    baseText: "",
    id: Math.floor(Math.random() * 1000),
  });

  const { model, workbookState, refresh } = props;
  useEffect(() => {
    const canvasRef = canvasElement.current;
    const columnGuideRef = columnResizeGuide.current;
    const rowGuideRef = rowResizeGuide.current;
    const columnHeadersRef = columnHeaders.current;
    const worksheetRef = worksheetElement.current;

    const outline = cellOutline.current;
    const handle = cellOutlineHandle.current;
    const area = areaOutline.current;
    const extendTo = extendToOutline.current;

    if (
      !canvasRef ||
      !columnGuideRef ||
      !rowGuideRef ||
      !columnHeadersRef ||
      !worksheetRef ||
      !outline ||
      !handle ||
      !area ||
      !extendTo ||
      !scrollElement.current
    )
      return;
    model.setWindowWidth(worksheetRef.clientWidth);
    model.setWindowHeight(worksheetRef.clientHeight);
    const canvas = new WorksheetCanvas({
      width: worksheetRef.clientWidth,
      height: worksheetRef.clientHeight,
      model,
      workbookState,
      elements: {
        canvas: canvasRef,
        columnGuide: columnGuideRef,
        rowGuide: rowGuideRef,
        columnHeaders: columnHeadersRef,
        cellOutline: outline,
        cellOutlineHandle: handle,
        areaOutline: area,
        extendToOutline: extendTo,
      },
      onColumnWidthChanges(sheet, column, width) {
        model.setColumnWidth(sheet, column, width);
        worksheetCanvas.current?.renderSheet();
      },
      onRowHeightChanges(sheet, row, height) {
        model.setRowHeight(sheet, row, height);
        worksheetCanvas.current?.renderSheet();
      },
    });
    const scrollX = model.getScrollX();
    const scrollY = model.getScrollY();
    const [sheetWidth, sheetHeight] = [scrollX + 100_000, scrollY + 500_000]; //canvas.getSheetDimensions();
    if (spacerElement.current) {
      spacerElement.current.style.height = `${sheetHeight}px`;
      spacerElement.current.style.width = `${sheetWidth}px`;
    }
    const left = scrollElement.current.scrollLeft;
    const top = scrollElement.current.scrollTop;
    if (scrollX !== left) {
      ignoreScrollEventRef.current = true;
      scrollElement.current.scrollLeft = scrollX;
      setTimeout(() => {
        ignoreScrollEventRef.current = false;
      }, 0);
    }

    if (scrollY !== top) {
      ignoreScrollEventRef.current = true;
      scrollElement.current.scrollTop = scrollY;
      setTimeout(() => {
        ignoreScrollEventRef.current = false;
      }, 0);
    }

    canvas.renderSheet();
    worksheetCanvas.current = canvas;
  });

  const sheetNames = model
    .getWorksheetsProperties()
    .map((s: { name: string }) => s.name);

  const {
    onPointerMove,
    onPointerDown,
    onPointerHandleDown,
    onPointerUp,
    // onContextMenu,
  } = usePointer({
    onCellSelected: (cell: Cell, event: React.MouseEvent) => {
      event.preventDefault();
      event.stopPropagation();
      model.setSelectedCell(cell.row, cell.column);
      refresh();
    },
    onAreaSelecting: (cell: Cell) => {
      const canvas = worksheetCanvas.current;
      if (!canvas) {
        return;
      }
      const { row, column } = cell;
      model.onAreaSelecting(row, column);
      canvas.renderSheet();
    },
    onAreaSelected: () => {
      let styles = workbookState.getCopyStyles();
      if (styles && styles.length) {
        model.onPasteStyles(styles);
        const canvas = worksheetCanvas.current;
        if (!canvas) {
          return;
        }
        canvas.renderSheet();
      }
      workbookState.setCopyStyles(null);
      if (worksheetElement.current) {
        worksheetElement.current.style.cursor = "auto";
      }
    },
    onExtendToCell: (cell) => {
      const canvas = worksheetCanvas.current;
      if (!canvas) {
        return;
      }
      const { row, column } = cell;
      const {
        sheet,
        range: [rowStart, columnStart, rowEnd, columnEnd],
      } = model.getSelectedView();
      // We are either extending by rows or by columns
      // And we could be doing it in the positive direction (downwards or right)
      // or the negative direction (upwards or left)

      if (
        row > rowEnd &&
        ((column <= columnEnd && column >= columnStart) ||
          (column < columnStart && columnStart - column < row - rowEnd) ||
          (column > columnEnd && column - columnEnd < row - rowEnd))
      ) {
        // rows downwards
        const area = {
          type: AreaType.rowsDown,
          rowStart: rowEnd + 1,
          rowEnd: row,
          columnStart,
          columnEnd,
        };
        workbookState.setExtendToArea(area);
        canvas.renderSheet();
      } else if (
        row < rowStart &&
        ((column <= columnEnd && column >= columnStart) ||
          (column < columnStart && columnStart - column < rowStart - row) ||
          (column > columnEnd && column - columnEnd < rowStart - row))
      ) {
        // rows upwards
        const area = {
          type: AreaType.rowsUp,
          rowStart: row,
          rowEnd: rowStart,
          columnStart,
          columnEnd,
        };
        workbookState.setExtendToArea(area);
        canvas.renderSheet();
      } else if (
        column > columnEnd &&
        ((row <= rowEnd && row >= rowStart) ||
          (row < rowStart && rowStart - row < column - columnEnd) ||
          (row > rowEnd && row - rowEnd < column - columnEnd))
      ) {
        // columns right
        const area = {
          type: AreaType.columnsRight,
          rowStart,
          rowEnd,
          columnStart: columnEnd + 1,
          columnEnd: column,
        };
        workbookState.setExtendToArea(area);
        canvas.renderSheet();
      } else if (
        column < columnStart &&
        ((row <= rowEnd && row >= rowStart) ||
          (row < rowStart && rowStart - row < columnStart - column) ||
          (row > rowEnd && row - rowEnd < columnStart - column))
      ) {
        // columns left
        const area = {
          type: AreaType.columnsLeft,
          rowStart,
          rowEnd,
          columnStart: column,
          columnEnd: columnStart,
        };
        workbookState.setExtendToArea(area);
        canvas.renderSheet();
      }
    },
    onExtendToEnd: () => {
      const canvas = worksheetCanvas.current;
      if (!canvas) {
        return;
      }
      const { sheet, range } = model.getSelectedView();
      const extendedArea = workbookState.getExtendToArea();
      if (!extendedArea) {
        return;
      }
      const rowStart = Math.min(range[0], range[2]);
      const height = Math.abs(range[2] - range[0]) + 1;
      const width = Math.abs(range[3] - range[1]) + 1;
      const columnStart = Math.min(range[1], range[3]);

      const area = { sheet, row: rowStart, column: columnStart, width, height };

      switch (extendedArea.type) {
        case AreaType.rowsDown:
          model.autoFillRows(area, extendedArea.rowEnd);
          break;
        case AreaType.rowsUp: {
          model.autoFillRows(area, extendedArea.rowStart);
          break;
        }
        case AreaType.columnsRight: {
          model.autoFillColumns(area, extendedArea.columnEnd);
          break;
        }
        case AreaType.columnsLeft: {
          model.autoFillColumns(area, extendedArea.columnStart);
          break;
        }
      }
      workbookState.clearExtendToArea();
      canvas.renderSheet();
    },
    canvasElement,
    worksheetElement,
    worksheetCanvas,
    // rowContextMenuAnchorElement,
    // columnContextMenuAnchorElement,
    // onRowContextMenu,
    // onColumnContextMenu,
  });

  const onScroll = (_event: any): void => {
    if (!scrollElement.current || !worksheetCanvas.current) {
      return;
    }
    if (ignoreScrollEventRef.current) {
      // Programmatic scroll ignored
      return;
    }
    const left = scrollElement.current.scrollLeft;
    const top = scrollElement.current.scrollTop;

    worksheetCanvas.current.setScrollPosition({ left, top });
    worksheetCanvas.current.renderSheet();
  };

  const { row, column, sheet: selectedSheet } = model.getSelectedView();

  return (
    // <EditorContext.Provider value={{editorContext}}>
    <Wrapper ref={scrollElement} onScroll={onScroll} className="scroll">
      <Spacer ref={spacerElement} />
      <SheetContainer
        className="sheet-container"
        ref={worksheetElement}
        onPointerDown={(event) => {
          if (isEditing === true && editorContext.mode !== "insert") {
            setEditing(false);
            model.setUserInput(
              selectedSheet,
              row,
              column,
              editorContext.baseText
            );
          }
          onPointerDown(event);
        }}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onDoubleClick={(event) => {
          const { sheet, row, column } = model.getSelectedView();
          const text = model.getCellContent(sheet, row, column) || "";
          console.log("dbclick", text);

          workbookState.startEditing("cell", `${text}`);
          setEditorContext((c: EditorState) => {
            console.log("text", text, c.id);
            return {
              mode: c.mode,
              insertRange: c.insertRange,
              baseText: text,
              dontChange: true,
              id: c.id,
            };
          });

          setEditing(true);
          event.stopPropagation();
          event.preventDefault();
          // refresh();
        }}
      >
        <SheetCanvas ref={canvasElement} />
        <CellOutline ref={cellOutline}>
          {
            <Editor
              minimalWidth={200}
              minimalHeight={90}
              textColor="#333"
              getStyledText={(text: string, insertRangeText: string) => {
                return getFormulaHTML(
                  text,
                  0,
                  sheetNames,
                  editorContext.insertRange,
                  insertRangeText
                );
              }}
              onEditEnd={(text: string) => {
                console.log(text);
                setEditing(false);
                model.setUserInput(selectedSheet, row, column, text);
              }}
              originalText={
                model.getCellContent(selectedSheet, row, column) || ""
              }
              display={isEditing}
              cell={{ sheet: selectedSheet, row, column }}
              sheetNames={sheetNames}
            />
          }
        </CellOutline>
        <AreaOutline ref={areaOutline} />
        <ExtendToOutline ref={extendToOutline} />
        <CellOutlineHandle
          ref={cellOutlineHandle}
          onPointerDown={onPointerHandleDown}
        />
        <ColumnResizeGuide ref={columnResizeGuide} />
        <RowResizeGuide ref={rowResizeGuide} />
        <ColumnHeaders ref={columnHeaders} />
      </SheetContainer>
    </Wrapper>
    // </EditorContext.Provider>
  );
}

const Spacer = styled("div")`
  position: absolute;
  height: 5000px;
  width: 5000px;
`;

const SheetContainer = styled("div")`
  position: sticky;
  top: 0px;
  left: 0px;
  height: 100%;

  .column-resize-handle {
    position: absolute;
    top: 0px;
    width: 3px;
    opacity: 0;
    background: ${outlineColor};
    border-radius: 5px;
    cursor: col-resize;
  }

  .column-resize-handle:hover {
    opacity: 1;
  }
  .row-resize-handle {
    position: absolute;
    left: 0px;
    height: 3px;
    opacity: 0;
    background: ${outlineColor};
    border-radius: 5px;
    cursor: row-resize;
  }

  .row-resize-handle:hover {
    opacity: 1;
  }
`;

const Wrapper = styled("div")({
  position: "absolute",
  overflow: "scroll",
  top: 71,
  left: 0,
  right: 0,
  bottom: 41,
});

const SheetCanvas = styled("canvas")`
  position: relative;
  top: 0px;
  left: 0px;
  right: 0px;
  bottom: 40px;
`;

const ColumnResizeGuide = styled("div")`
  position: absolute;
  top: 0px;
  display: none;
  height: 100%;
  width: 0px;
  border-left: 1px dashed ${outlineColor};
`;

const ColumnHeaders = styled("div")`
  position: absolute;
  left: 0px;
  top: 0px;
  overflow: hidden;
  display: flex;
  & .column-header {
    display: inline-block;
    text-align: center;
    overflow: hidden;
    height: 100%;
    user-select: none;
  }
`;

const RowResizeGuide = styled("div")`
  position: absolute;
  display: none;
  left: 0px;
  height: 0px;
  width: 100%;
  border-top: 1px dashed ${outlineColor};
`;

const AreaOutline = styled("div")`
  position: absolute;
  border: 1px solid ${outlineColor};
  border-radius: 3px;
  background-color: ${outlineBackgroundColor};
`;

const CellOutline = styled("div")`
  position: absolute;
  border: 2px solid ${outlineColor};
  border-radius: 3px;
  word-break: break-word;
  font-size: 13px;
  display: flex;
`;

const CellOutlineHandle = styled("div")`
  position: absolute;
  width: 5px;
  height: 5px;
  background: ${outlineColor};
  cursor: crosshair;
  // border: 1px solid white;
  border-radius: 1px;
`;

const ExtendToOutline = styled("div")`
  position: absolute;
  border: 1px dashed ${outlineColor};
  border-radius: 3px;
`;

export default Worksheet;
