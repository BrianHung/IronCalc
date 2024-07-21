import Toolbar from "./toolbar";
import FormulaBar from "./formulabar";
import Navigation from "./navigation/navigation";
import Worksheet from "./worksheet";
import { styled } from "@mui/material/styles";
import { useEffect, useRef, useState } from "react";
import useKeyboardNavigation from "./useKeyboardNavigation";
import { NavigationKey, getCellAddress } from "./WorksheetCanvas/util";
import { LAST_COLUMN, LAST_ROW } from "./WorksheetCanvas/constants";
import { WorkbookState } from "./workbookState";
import { BorderOptions, Model, WorksheetProperties } from "@ironcalc/wasm";

const Workbook = (props: { model: Model; workbookState: WorkbookState }) => {
  const { model, workbookState } = props;
  const rootRef = useRef<HTMLDivElement>(null);
  const [_redrawId, setRedrawId] = useState(0);
  const info = model
    .getWorksheetsProperties()
    .map(({ name, color, sheet_id }: WorksheetProperties) => {
      return { name, color: color ? color : "#FFF", sheetId: sheet_id };
    });

  const onRedo = () => {
    model.redo();
    setRedrawId((id) => id + 1);
  };

  const onUndo = () => {
    model.undo();
    setRedrawId((id) => id + 1);
  };

  const updateRangeStyle = (stylePath: string, value: string) => {
    const {
      sheet,
      range: [rowStart, columnStart, rowEnd, columnEnd],
    } = model.getSelectedView();
    const row = Math.min(rowStart, rowEnd);
    const column = Math.min(columnStart, columnEnd);
    const range = {
      sheet,
      row,
      column,
      width: Math.abs(columnEnd - columnStart) + 1,
      height: Math.abs(rowEnd - rowStart) + 1,
    };
    model.updateRangeStyle(range, stylePath, value);
    setRedrawId((id) => id + 1);
  };

  const onToggleUnderline = (value: boolean) => {
    updateRangeStyle("font.u", `${value}`);
  };

  const onToggleItalic = (value: boolean) => {
    updateRangeStyle("font.i", `${value}`);
  };

  const onToggleBold = (value: boolean) => {
    updateRangeStyle("font.b", `${value}`);
  };

  const onToggleStrike = (value: boolean) => {
    updateRangeStyle("font.strike", `${value}`);
  };

  const onToggleHorizontalAlign = (value: string) => {
    updateRangeStyle("alignment.horizontal", value);
  };

  const onToggleVerticalAlign = (value: string) => {
    updateRangeStyle("alignment.vertical", value);
  };

  const onTextColorPicked = (hex: string) => {
    updateRangeStyle("font.color", hex);
  };

  const onFillColorPicked = (hex: string) => {
    updateRangeStyle("fill.fg_color", hex);
  };

  const onNumberFormatPicked = (numberFmt: string) => {
    updateRangeStyle("num_fmt", numberFmt);
  };

  const onCopyStyles = () => {
    const {
      sheet,
      range: [rowStart, columnStart, rowEnd, columnEnd],
    } = model.getSelectedView();
    const row1 = Math.min(rowStart, rowEnd);
    const column1 = Math.min(columnStart, columnEnd);
    const row2 = Math.max(rowStart, rowEnd);
    const column2 = Math.max(columnStart, columnEnd);

    const styles = [];
    for (let row = row1; row <= row2; row++) {
      const styleRow = [];
      for (let column = column1; column <= column2; column++) {
        styleRow.push(model.getCellStyle(sheet, row, column));
      }
      styles.push(styleRow);
    }
    console.log("set styles", styles);
    workbookState.setCopyStyles(styles);
    const el = rootRef.current?.getElementsByClassName("sheet-container")[0];
    if (el) {
      (el as HTMLElement).style.cursor =
        `url('data:image/svg+xml;utf8,<svg data-v-56bd7dfc="" xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="lucide lucide-paintbrush-vertical"><path d="M10 2v2"></path><path d="M14 2v4"></path><path d="M17 2a1 1 0 0 1 1 1v9H6V3a1 1 0 0 1 1-1z"></path><path d="M6 12a1 1 0 0 0-1 1v1a2 2 0 0 0 2 2h2a1 1 0 0 1 1 1v2.9a2 2 0 1 0 4 0V17a1 1 0 0 1 1-1h2a2 2 0 0 0 2-2v-1a1 1 0 0 0-1-1"></path></svg>'), auto`;
    }
  };

  // FIXME: My gut tells me that we should have only one on onKeyPressed function that goes to
  // the Rust end
  const { onKeyDown } = useKeyboardNavigation({
    onCellsDeleted: function (): void {
      const {
        sheet,
        range: [rowStart, columnStart, rowEnd, columnEnd],
      } = model.getSelectedView();
      const row = Math.min(rowStart, rowEnd);
      const column = Math.min(columnStart, columnEnd);

      const width = Math.abs(columnEnd - columnStart) + 1;
      const height = Math.abs(rowEnd - rowStart) + 1;
      model.rangeClearContents(
        sheet,
        row,
        column,
        row + height,
        column + width
      );
      setRedrawId((id) => id + 1);
    },
    onExpandAreaSelectedKeyboard: function (
      key: "ArrowRight" | "ArrowLeft" | "ArrowUp" | "ArrowDown"
    ): void {
      model.onExpandSelectedRange(key);
      setRedrawId((id) => id + 1);
    },
    onEditKeyPressStart: function (initText: string): void {
      console.log(initText);
      throw new Error("Function not implemented.");
    },
    onCellEditStart: function (): void {
      throw new Error("Function not implemented.");
    },
    onBold: () => {
      let { sheet, row, column } = model.getSelectedView();
      let value = !model.getCellStyle(sheet, row, column).font.b;
      onToggleBold(!value);
    },
    onItalic: () => {
      let { sheet, row, column } = model.getSelectedView();
      let value = !model.getCellStyle(sheet, row, column).font.i;
      onToggleItalic(!value);
    },
    onUnderline: () => {
      let { sheet, row, column } = model.getSelectedView();
      let value = !model.getCellStyle(sheet, row, column).font.u;
      onToggleUnderline(!value);
    },
    onNavigationToEdge: function (direction: NavigationKey): void {
      console.log(direction);
      // const newSelectedCell = model.getNavigationEdge(
      //   key,
      //   selectedSheet,
      //   selectedCell.row,
      //   selectedCell.column,
      //   canvas.lastRow,
      //   canvas.lastColumn,
      // );
      setRedrawId((id) => id + 1);
    },
    onPageDown: function (): void {
      model.onPageDown();
      setRedrawId((id) => id + 1);
    },
    onPageUp: function (): void {
      model.onPageUp();
      setRedrawId((id) => id + 1);
    },
    onArrowDown: function (): void {
      model.onArrowDown();
      setRedrawId((id) => id + 1);
    },
    onArrowUp: function (): void {
      model.onArrowUp();
      setRedrawId((id) => id + 1);
    },
    onArrowLeft: function (): void {
      model.onArrowLeft();
      setRedrawId((id) => id + 1);
    },
    onArrowRight: function (): void {
      model.onArrowRight();
      setRedrawId((id) => id + 1);
    },
    onKeyHome: function (): void {
      const view = model.getSelectedView();
      const cell = model.getSelectedCell();
      model.setSelectedCell(cell[1], 1);
      model.setTopLeftVisibleCell(view.top_row, 1);
      setRedrawId((id) => id + 1);
    },
    onKeyEnd: function (): void {
      const view = model.getSelectedView();
      const cell = model.getSelectedCell();
      model.setSelectedCell(cell[1], LAST_COLUMN);
      model.setTopLeftVisibleCell(view.top_row, LAST_COLUMN - 5);
      setRedrawId((id) => id + 1);
    },
    onUndo: function (): void {
      model.undo();
      setRedrawId((id) => id + 1);
    },
    onRedo: function (): void {
      model.redo();
      setRedrawId((id) => id + 1);
    },
    onNextSheet: function (): void {
      const nextSheet = model.getSelectedSheet() + 1;
      if (nextSheet >= model.getWorksheetsProperties().length) {
        model.setSelectedSheet(0);
      } else {
        model.setSelectedSheet(nextSheet);
      }
    },
    onPreviousSheet: function (): void {
      const nextSheet = model.getSelectedSheet() - 1;
      if (nextSheet < 0) {
        model.setSelectedSheet(model.getWorksheetsProperties().length - 1);
      } else {
        model.setSelectedSheet(nextSheet);
      }
    },
    root: rootRef,
  });

  useEffect(() => {
    if (!rootRef.current) {
      return;
    }
    rootRef.current.focus();
  });

  const {
    sheet,
    row,
    column,
    range: [rowStart, columnStart, rowEnd, columnEnd],
  } = model.getSelectedView();

  const cellAddress = getCellAddress(
    { rowStart, rowEnd, columnStart, columnEnd },
    { row, column }
  );
  const formulaValue = model.getCellContent(sheet, row, column);

  const style = model.getCellStyle(sheet, row, column);

  return (
    <Container ref={rootRef} onKeyDown={onKeyDown} tabIndex={0}>
      <Toolbar
        canUndo={model.canUndo()}
        canRedo={model.canRedo()}
        onRedo={onRedo}
        onUndo={onUndo}
        onToggleUnderline={onToggleUnderline}
        onToggleBold={onToggleBold}
        onToggleItalic={onToggleItalic}
        onToggleStrike={onToggleStrike}
        onToggleHorizontalAlign={onToggleHorizontalAlign}
        onToggleVerticalAlign={onToggleVerticalAlign}
        onCopyStyles={onCopyStyles}
        onTextColorPicked={onTextColorPicked}
        onFillColorPicked={onFillColorPicked}
        onNumberFormatPicked={onNumberFormatPicked}
        onBorderChanged={function (border: BorderOptions): void {
          const {
            sheet,
            range: [rowStart, columnStart, rowEnd, columnEnd],
          } = model.getSelectedView();
          const row = Math.min(rowStart, rowEnd);
          const column = Math.min(columnStart, columnEnd);

          const width = Math.abs(columnEnd - columnStart) + 1;
          const height = Math.abs(rowEnd - rowStart) + 1;
          const borderArea = {
            type: border.border,
            item: border,
          };
          model.setAreaWithBorder(
            { sheet, row, column, width, height },
            borderArea
          );
          setRedrawId((id) => id + 1);
        }}
        fillColor={style.fill.fg_color || "#FFF"}
        fontColor={style.font.color}
        bold={style.font.b}
        underline={style.font.u}
        italic={style.font.i}
        strike={style.font.strike}
        horizontalAlign={
          style.alignment ? style.alignment.horizontal : "general"
        }
        verticalAlign={style.alignment ? style.alignment.vertical : "center"}
        canEdit={true}
        numFmt={""}
        showGridLines={model.getShowGridLines(sheet)}
        onToggleShowGridLines={(show) => {
          model.setShowGridLines(sheet, show);
          setRedrawId((id) => id + 1);
        }}
      />
      <FormulaBar
        cellAddress={cellAddress}
        formulaValue={formulaValue}
        onChange={(value) => {
          console.log('set', sheet, row, column, value);
          model.setUserInput(sheet, row, column, value);
          setRedrawId((id) => id + 1);
        }}
      />
      <Worksheet
        model={model}
        workbookState={workbookState}
        refresh={(): void => {
          setRedrawId((id) => id + 1);
        }}
      />
      <Navigation
        sheets={info}
        selectedIndex={model.getSelectedSheet()}
        onSheetSelected={function (sheet: number): void {
          model.setSelectedSheet(sheet);
          setRedrawId((value) => value + 1);
        }}
        onAddBlankSheet={function (): void {
          model.newSheet();
        }}
        onSheetColorChanged={function (hex: string): void {
          try {
            model.setSheetColor(model.getSelectedSheet(), hex);
          } catch (e) {
            alert(`${e}`);
          }
        }}
        onSheetRenamed={function (name: string): void {
          try {
            model.renameSheet(model.getSelectedSheet(), name);
          } catch (e) {
            alert(`${e}`);
          }
        }}
        onSheetDeleted={function (): void {
          model.deleteSheet(model.getSelectedSheet());
        }}
      />
    </Container>
  );
};

const Container = styled("div")`
  display: flex;
  flex-direction: column;
  height: 100%;
  font-family: ${({ theme }) => theme.typography.fontFamily};

  &:focus {
    outline: none;
  }
`;

export default Workbook;
