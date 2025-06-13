import test from 'node:test';
import assert from 'node:assert'
import { Model } from "../pkg/wasm.js";
import { setTimeout } from 'node:timers/promises';

const DEFAULT_ROW_HEIGHT = 28;

test('Frozen rows and columns', () => {
    let model = new Model('Workbook1', 'en', 'UTC');
    assert.strictEqual(model.getFrozenRowsCount(0), 0);
    assert.strictEqual(model.getFrozenColumnsCount(0), 0);

    model.setFrozenColumnsCount(0, 4);
    model.setFrozenRowsCount(0, 3)

    assert.strictEqual(model.getFrozenRowsCount(0), 3);
    assert.strictEqual(model.getFrozenColumnsCount(0), 4);
});

test('Row height', () => {
    let model = new Model('Workbook1', 'en', 'UTC');
    assert.strictEqual(model.getRowHeight(0, 3), DEFAULT_ROW_HEIGHT);

    model.setRowsHeight(0, 3, 3, 32);
    assert.strictEqual(model.getRowHeight(0, 3), 32);

    model.undo();
    assert.strictEqual(model.getRowHeight(0, 3), DEFAULT_ROW_HEIGHT);

    model.redo();
    assert.strictEqual(model.getRowHeight(0, 3), 32);

    model.setRowsHeight(0, 3, 3, 320);
    assert.strictEqual(model.getRowHeight(0, 3), 320);
});

test('Evaluates correctly', (t) => {
    const model = new Model('Workbook1', 'en', 'UTC');
    model.setUserInput(0, 1, 1, "23");
    model.setUserInput(0, 1, 2, "=A1*3+1");

    const result = model.getFormattedCellValue(0, 1, 2);
    assert.strictEqual(result, "70");
});

test('Styles work', () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    let style = model.getCellStyle(0, 1, 1);
    assert.deepEqual(style, {
        num_fmt: 'general',
        fill: { pattern_type: 'none' },
        font: {
            sz: 13,
            color: '#000000',
            name: 'Calibri',
            family: 2,
            scheme: 'minor'
        },
        border: {},
        quote_prefix: false
    });
    model.setUserInput(0, 1, 1, "'=1+1");
    style = model.getCellStyle(0, 1, 1);
    assert.deepEqual(style, {
        num_fmt: 'general',
        fill: { pattern_type: 'none' },
        font: {
            sz: 13,
            color: '#000000',
            name: 'Calibri',
            family: 2,
            scheme: 'minor'
        },
        border: {},
        quote_prefix: true
    });
});

test("add sheets", (t) => {
    const model = new Model('Workbook1', 'en', 'UTC');
    model.newSheet();
    model.renameSheet(1, "NewName");
    let props = model.getWorksheetsProperties();
    assert.deepEqual(props, [{
        name: 'Sheet1',
        sheet_id: 1,
        state: 'visible'
    },
    {
        name: 'NewName',
        sheet_id: 2,
        state: 'visible'
    }
    ]);
});

test("newSheet returns sheet result", (t) => {
    const model = new Model('Workbook1', 'en', 'UTC');
    
    // Test first new sheet - should be at index 1
    const result1 = model.newSheet();
    console.log(result1);
    assert.strictEqual(result1.name, "Sheet2");
    assert.strictEqual(result1.sheet_index, 1);  // This is the sheet index (position)
    
    // Test second new sheet - should be at index 2
    const result2 = model.newSheet();
    assert.strictEqual(result2.name, "Sheet3");
    assert.strictEqual(result2.sheet_index, 2);  // This is the sheet index (position)
    
    // Verify we can use the returned index with other API methods
    model.renameSheet(result1.sheet_index, "FirstNewSheet");
    model.renameSheet(result2.sheet_index, "SecondNewSheet");
    
    // Verify the sheets actually exist and were renamed
    const props = model.getWorksheetsProperties();
    assert.strictEqual(props.length, 3);
    assert.strictEqual(props[1].name, "FirstNewSheet");
    assert.strictEqual(props[2].name, "SecondNewSheet");
});

test("invalid sheet index throws an exception", () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    assert.throws(() => {
        model.setRowsHeight(1, 1, 1, 100);
    }, {
        name: 'Error',
        message: 'Invalid sheet index',
    });
});

test('invalid column throws an exception', () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    assert.throws(() => {
        model.setRowsHeight(0, -1, 0, 100);
    }, {
        name: 'Error',
        message: "Row number '-1' is not valid.",
    });
});

test('floating column numbers get truncated', () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    model.setRowsHeight(0.8, 5.2, 5.5, 100.5);

    assert.strictEqual(model.getRowHeight(0.11, 5.99), 100.5);
    assert.strictEqual(model.getRowHeight(0, 5), 100.5);
});

test('autofill', () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    model.setUserInput(0, 1, 1, "23");
    model.autoFillRows({sheet: 0, row: 1, column: 1, width: 1, height: 1}, 2);

    const result = model.getFormattedCellValue(0, 2, 1);
    assert.strictEqual(result, "23");
});

test('track changed cells - basic update', () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    model.setUserInput(0, 1, 1, "10");
    model.setUserInput(0, 1, 2, "=A1*2");
    model.evaluate();
    const changedCells = model.getChangedCells();
    assert.strictEqual(changedCells.length, 1, 'Changed cells should include directly set cell and dependent cell');
    assert.deepEqual(changedCells[0], { sheet: 0, row: 1, column: 2 }, 'Second changed cell should be B1');
});

test('getRecentDiffs returns recent diffs without modifying queue', () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    // Perform some actions to generate diffs
    model.setUserInput(0, 1, 1, "42");
    model.setUserInput(0, 1, 2, "=A1*2");
    
    // Get recent diffs
    const diffs = model.getRecentDiffs();
    assert.strictEqual(diffs.length > 0, true, 'Diffs array should not be empty after actions');
    
    // Check structure of diffs - regular operations are marked as "Redo" type
    const firstDiff = diffs[0];
    assert.strictEqual(firstDiff.type, 'Redo', 'Regular operations should be of type Redo');
    assert.strictEqual(Array.isArray(firstDiff.list), true, 'Diff entry should have a list of diffs');
    assert.strictEqual(firstDiff.list.length > 0, true, 'Diff list should not be empty');
    
    // Look for SetCellValue diff in any of the diff entries
    let foundSetCellValue = false;
    for (const diffEntry of diffs) {
        const setCellDiff = diffEntry.list.find(d => d.SetCellValue && d.SetCellValue.row === 1 && d.SetCellValue.column === 1);
        if (setCellDiff) {
            assert.strictEqual(setCellDiff.SetCellValue.new_value, '42', 'New value for A1 should be 42');
            foundSetCellValue = true;
            break;
        }
    }
    assert.ok(foundSetCellValue, 'Should find a SetCellValue diff for cell A1 somewhere in the diffs');
    
    // Verify queue is not modified by checking again
    const diffsAgain = model.getRecentDiffs();
    assert.strictEqual(diffsAgain.length, diffs.length, 'Queue length should remain the same after multiple calls');
    assert.deepStrictEqual(diffsAgain, diffs, 'Queue contents should remain unchanged after multiple calls');
});

test('getRecentDiffs captures style changes', () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    // Perform a style change
    model.updateRangeStyle({ sheet: 0, row: 1, column: 1, width: 1, height: 1 }, 'font.b', 'true');
    
    // Get recent diffs
    const diffs = model.getRecentDiffs();
    assert.strictEqual(diffs.length > 0, true, 'Diffs array should not be empty after style change');
    
    // Look for SetCellStyle diff in any of the diff entries 
    let foundStyleDiff = false;
    for (const diffEntry of diffs) {
        const styleDiff = diffEntry.list.find(d => d.SetCellStyle);
        if (styleDiff) {
            assert.strictEqual(styleDiff.SetCellStyle.sheet, 0, 'Sheet index should be 0');
            assert.strictEqual(styleDiff.SetCellStyle.row, 1, 'Row should be 1');
            assert.strictEqual(styleDiff.SetCellStyle.column, 1, 'Column should be 1');
            assert.ok(styleDiff.SetCellStyle.new_value.font.b, 'New style should have bold set to true');
            foundStyleDiff = true;
            break;
        }
    }
    assert.ok(foundStyleDiff, 'Should find a SetCellStyle diff after style update');
});

test('getRecentDiffs captures undo and redo diffs', () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    // Perform an action and undo it
    model.setUserInput(0, 1, 1, "100");
    model.undo();
    
    // Get recent diffs
    const diffs = model.getRecentDiffs();
    assert.strictEqual(diffs.length > 0, true, 'Diffs array should not be empty after undo');
    
    // Check for Undo type in diffs
    const undoDiff = diffs.find(d => d.type === 'Undo');
    assert.ok(undoDiff, 'Should find an Undo diff entry after undo operation');
    assert.strictEqual(undoDiff.list.length > 0, true, 'Undo diff list should not be empty');
    
    // Redo the action
    model.redo();
    const diffsAfterRedo = model.getRecentDiffs();
    const redoDiff = diffsAfterRedo.find(d => d.type === 'Redo');
    assert.ok(redoDiff, 'Should find a Redo diff entry after redo operation');
});

test('getRecentDiffs captures setCellValue diff', () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    // Set a cell value to generate a SetCellValue diff
    model.setUserInput(0, 2, 3, "99");
    
    // Get recent diffs
    const diffs = model.getRecentDiffs();
    assert.strictEqual(diffs.length > 0, true, 'Diffs array should not be empty after setting cell value');
    
    // Look for SetCellValue diff in any of the diff entries
    let foundSetCellDiff = false;
    for (const diffEntry of diffs) {
        const setCellDiff = diffEntry.list.find(d => d.SetCellValue);
        if (setCellDiff) {
            assert.strictEqual(setCellDiff.SetCellValue.sheet, 0, 'Sheet index should be 0');
            assert.strictEqual(setCellDiff.SetCellValue.row, 2, 'Row should be 2');
            assert.strictEqual(setCellDiff.SetCellValue.column, 3, 'Column should be 3');
            assert.strictEqual(setCellDiff.SetCellValue.new_value, '99', 'New value should be 99');
            foundSetCellDiff = true;
            break;
        }
    }
    assert.ok(foundSetCellDiff, 'Should find a SetCellValue diff after setting cell value');
});
test("getSheetDimensions", () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    
    // Test empty sheet - should return default dimensions
    let dimensions = model.getSheetDimensions(0);
    assert.deepEqual(dimensions, {
        min_row: 1,
        max_row: 1,
        min_column: 1,
        max_column: 1
    });
    
    // Add a single cell at A1
    model.setUserInput(0, 1, 1, "Hello");
    dimensions = model.getSheetDimensions(0);
    assert.deepEqual(dimensions, {
        min_row: 1,
        max_row: 1,
        min_column: 1,
        max_column: 1
    });
    
    // Add another cell to expand the range
    model.setUserInput(0, 5, 8, "World");
    dimensions = model.getSheetDimensions(0);
    assert.deepEqual(dimensions, {
        min_row: 1,
        max_row: 5,
        min_column: 1,
        max_column: 8
    });
});

test('track changed cells - circular dependency with external dependent', () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    // Setup circular dependency: A1 -> B1 -> C1 -> A1
    model.setUserInput(0, 1, 1, "=B1");
    model.setUserInput(0, 1, 2, "=C1");
    model.setUserInput(0, 1, 3, "=A1");
    // Setup external dependent: D1 depends on A1
    model.setUserInput(0, 1, 4, "=A1+1");
    // Evaluate to set initial state
    model.evaluate();
    // Update A1 to trigger circular dependency error
    model.setUserInput(0, 1, 1, "=B1+10");
    model.evaluate();
    // Get changed cells
    const changedCells = model.getChangedCells();
    // Check if dependent cells are tracked as changed, excluding A1 which was directly updated
    assert.strictEqual(changedCells.some(c => c.sheet === 0 && c.row === 1 && c.column === 2), true, 'B1 should be tracked as changed due to circular dependency');
    assert.strictEqual(changedCells.some(c => c.sheet === 0 && c.row === 1 && c.column === 3), true, 'C1 should be tracked as changed due to circular dependency');
    assert.strictEqual(changedCells.some(c => c.sheet === 0 && c.row === 1 && c.column === 4), true, 'D1 should be tracked as changed due to dependency on A1');
});

test('track changed cells - multi-sheet cascading with defined name', () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    // Add additional sheets
    model.newSheet();
    model.renameSheet(1, "Sheet2");
    model.newSheet();
    model.renameSheet(2, "Sheet3");
    // Define a name 'TotalSales' for Sheet1!A1:A2
    model.newDefinedName("TotalSales", 0, "=Sheet1!A1:A2");
    // Set values in Sheet1
    model.setUserInput(0, 1, 1, "100");
    model.setUserInput(0, 2, 1, "200");
    // Set formula in Sheet2 using defined name
    model.setUserInput(1, 2, 2, "=SUM(TotalSales)");
    // Set formula in Sheet3 referencing Sheet2!B2
    model.setUserInput(2, 3, 3, "=Sheet2!B2*2");
    // Evaluate initial state
    model.evaluate();
    // Update Sheet1!A1 to trigger cascading changes
    model.setUserInput(0, 1, 1, "150");
    model.evaluate();
    // Get changed cells
    const changedCells = model.getChangedCells();
    // Verify only dependent cells are tracked, excluding Sheet1!A1 which was directly updated
    assert.strictEqual(changedCells.some(c => c.sheet === 1 && c.row === 2 && c.column === 2), true, 'Sheet2!B2 should be tracked as changed');
    assert.strictEqual(changedCells.some(c => c.sheet === 2 && c.row === 3 && c.column === 3), true, 'Sheet3!C3 should be tracked as changed');
});

test('track changed cells - move row updates formulas', () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    model.setUserInput(0, 1, 1, "10");
    model.setUserInput(0, 2, 2, "=A1*2");
    model.evaluate();
    assert.strictEqual(model.getFormattedCellValue(0, 2, 2), "20");
    // Move row 1 to row 3
    model.insertRow(0, 1);
    model.insertRow(0, 1);
    model.evaluate();
    const changedCells = model.getChangedCells();
    assert.strictEqual(changedCells.length, 1, 'One cell should be marked as changed after row insertion');
    assert.deepEqual(changedCells[0], { sheet: 0, row: 4, column: 2 }, 'Changed cell should be B4 due to formula update after row shift');
});

test('track changed cells - move column updates formulas', () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    model.setUserInput(0, 1, 1, "5");
    model.setUserInput(0, 1, 2, "=A1+3");
    model.evaluate();
    assert.strictEqual(model.getFormattedCellValue(0, 1, 2), "8");
    // Insert a column before column 1, shifting existing columns
    model.insertColumn(0, 1);
    model.evaluate();
    const changedCells = model.getChangedCells();
    assert.strictEqual(changedCells.length, 1, 'One cell should be marked as changed after column insertion');
    assert.deepEqual(changedCells[0], { sheet: 0, row: 1, column: 3 }, 'Changed cell should be C1 due to formula update after column shift');
});
test('onDiffs', async () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    const events = [];
    
    model.onDiffs(diff => {
        events.push(diff);
    });
    
    model.setUserInput(0, 1, 1, 'test');
    await setTimeout(0);
    
    const expectedEvents = [
        {
            SetCellValue: {
                sheet: 0,
                row: 1,
                column: 1,
                new_value: 'test',
                old_value: undefined
            }
        },
    ];
    
    // Verify we got the expected number of events
    assert.strictEqual(events.length, expectedEvents.length, `Should have exactly ${expectedEvents.length} diff events`);
    
    // Compare each event with deep equality
    for (let i = 0; i < expectedEvents.length; i++) {
        assert.deepStrictEqual(events[i], expectedEvents[i], `Event ${i} should match expected diff`);
    }
});

test('saveToXLSX returns data', () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    const bytes = model.saveToXLSX();
    assert.ok(bytes instanceof Uint8Array);
    assert.ok(bytes.length > 0);
});

test('saveToICalc returns data', () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    const bytes = model.saveToICalc();
    assert.ok(bytes instanceof Uint8Array);
    assert.ok(bytes.length > 0);
});

test('onDiffs emits correct diff types for various operations', async () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    const events = [];
    
    model.onDiffs(diff => {
        events.push(diff);
    });
    
    // Test various operations that should emit different diff types
    model.setUserInput(0, 1, 1, '42');
    model.insertRow(0, 2);                                 
    model.setRowsHeight(0, 1, 1, 35);                     
    model.insertColumn(0, 2);                             
    model.setColumnsWidth(0, 1, 1, 120);                  
    model.newSheet();                                     
    model.renameSheet(1, "TestSheet");                    
    model.setFrozenRowsCount(0, 2);                       
    model.setFrozenColumnsCount(0, 3);                    
    
    // Allow any async operations to complete
    await setTimeout(0);
    
    const expectedEvents = [
        {
            SetCellValue: {
                sheet: 0,
                row: 1,
                column: 1,
                new_value: '42',
                old_value: undefined
            }
        },
        {
            InsertRow: {
                sheet: 0,
                row: 2
            }
        },
        {
            SetRowHeight: {
                sheet: 0,
                row: 1,
                new_value: 35,
                old_value: 28
            }
        },
        {
            InsertColumn: {
                sheet: 0,
                column: 2
            }
        },
        {
            SetColumnWidth: {
                sheet: 0,
                column: 1,
                new_value: 120,
                old_value: 125
            }
        },
        {
            NewSheet: {
                index: 1,
                name: 'Sheet2'
            }
        },
        {
            RenameSheet: {
                index: 1,
                old_value: 'Sheet2',
                new_value: 'TestSheet'
            }
        },
        {
            SetFrozenRowsCount: {
                sheet: 0,
                new_value: 2,
                old_value: 0
            }
        },
        {
            SetFrozenColumnsCount: {
                sheet: 0,
                new_value: 3,
                old_value: 0
            }
        }
    ];
    
    // Verify we got the expected number of events
    assert.strictEqual(events.length, expectedEvents.length, `Should have exactly ${expectedEvents.length} diff events`);
    
    // Compare each event with deep equality
    for (let i = 0; i < expectedEvents.length; i++) {
        assert.deepStrictEqual(events[i], expectedEvents[i], `Event ${i} should match expected diff`);
    }
});

test('onDiffs emits full diff objects for undo/redo operations', async () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    const events = [];
    
    model.onDiffs(diff => {
        events.push(diff);
    });
    
    // Perform initial operations
    model.setUserInput(0, 1, 1, 'Hello');
    model.setUserInput(0, 1, 2, 'World'); 
    model.insertRow(0, 2);
    
    // Test undo - should emit diffs for undoing operations
    model.undo();
    model.undo();
    
    // Test redo - should emit diffs for redoing operations  
    model.redo();
    model.redo();
    
    await setTimeout(0);
    
    const expectedEvents = [
        // Initial operations (3 events)
        {
            SetCellValue: {
                sheet: 0,
                row: 1,
                column: 1,
                new_value: 'Hello',
                old_value: undefined
            }
        },
        {
            SetCellValue: {
                sheet: 0,
                row: 1,
                column: 2,
                new_value: 'World',
                old_value: undefined
            }
        },
        {
            InsertRow: {
                sheet: 0,
                row: 2
            }
        },
        // Undo operations (2 events) - Note: these emit the same diff structures as the forward operations
        {
            InsertRow: {
                sheet: 0,
                row: 2
            }
        },
        {
            SetCellValue: {
                sheet: 0,
                row: 1,
                column: 2,
                new_value: 'World',
                old_value: undefined
            }
        },
        // Redo operations (2 events) - These also emit the same diff structures
        {
            SetCellValue: {
                sheet: 0,
                row: 1,
                column: 2,
                new_value: 'World',
                old_value: undefined
            }
        },
        {
            InsertRow: {
                sheet: 0,
                row: 2
            }
        }
    ];
    
    // Verify we got the expected number of events
    assert.strictEqual(events.length, expectedEvents.length, `Should have exactly ${expectedEvents.length} diff events`);
    
    // Compare each event with deep equality
    for (let i = 0; i < expectedEvents.length; i++) {
        assert.deepStrictEqual(events[i], expectedEvents[i], `Event ${i} should match expected diff`);
    }
});

test('onDiffs handles multiple subscribers and provides full diff objects', async () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    const events = [];
    
    model.onDiffs(diff => {
        events.push(diff);
    });
    
    // Perform complex operations that generate multiple diffs
    model.setUserInput(0, 1, 1, '=SUM(A2:A5)');
    model.setUserInput(0, 2, 1, '10');           
    model.setUserInput(0, 3, 1, '20');           
    model.setUserInput(0, 4, 1, '30');           
    
    // Test row operations
    model.insertRow(0, 2);
    model.deleteRow(0, 2);
    
    // Test range operations
    model.rangeClearContents(0, 2, 1, 3, 1);
    
    await setTimeout(0);
    
    const expectedEvents = [
        // SetUserInput operations (4 events)
        {
            SetCellValue: {
                sheet: 0,
                row: 1,
                column: 1,
                new_value: '=SUM(A2:A5)',
                old_value: undefined
            }
        },
        {
            SetCellValue: {
                sheet: 0,
                row: 2,
                column: 1,
                new_value: '10',
                old_value: undefined
            }
        },
        {
            SetCellValue: {
                sheet: 0,
                row: 3,
                column: 1,
                new_value: '20',
                old_value: undefined
            }
        },
        {
            SetCellValue: {
                sheet: 0,
                row: 4,
                column: 1,
                new_value: '30',
                old_value: undefined
            }
        },
        // Row operations (2 events)
        {
            InsertRow: {
                sheet: 0,
                row: 2
            }
        },
        {
            DeleteRow: {
                sheet: 0,
                row: 2,
                old_data: {
                    data: new Map(),
                    row: undefined
                }
            }
        },
        // Range clear operations (2 events)
        {
            CellClearContents: {
                sheet: 0,
                row: 2,
                column: 1,
                old_value: {
                    NumberCell: {
                        v: 10,
                        s: 0
                    }
                }
            }
        },
        {
            CellClearContents: {
                sheet: 0,
                row: 3,
                column: 1,
                old_value: {
                    NumberCell: {
                        v: 20,
                        s: 0
                    }
                }
            }
        }
    ];
    
    // Verify we got the expected number of events
    assert.strictEqual(events.length, expectedEvents.length, `Should have exactly ${expectedEvents.length} diff events`);
    
    // Compare each event with deep equality
    for (let i = 0; i < expectedEvents.length; i++) {
        assert.deepStrictEqual(events[i], expectedEvents[i], `Event ${i} should match expected diff`);
    }
});

test('onDiffs returns unregister function that works correctly', async () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    const events1 = [];
    const events2 = [];
    
    // Register two listeners
    const unregister1 = model.onDiffs(diff => {
        events1.push(diff);
    });
    
    const unregister2 = model.onDiffs(diff => {
        events2.push(diff);
    });
    
    // Both should be functions
    assert.strictEqual(typeof unregister1, 'function', 'onDiffs should return a function');
    assert.strictEqual(typeof unregister2, 'function', 'onDiffs should return a function');
    
    // Trigger some events
    model.setUserInput(0, 1, 1, 'Test');
    model.setUserInput(0, 1, 2, 'Test2');
    
    await setTimeout(0);
    
    // Both listeners should have received events
    assert.strictEqual(events1.length, 2, 'First listener should receive 2 events');
    assert.strictEqual(events2.length, 2, 'Second listener should receive 2 events');
    
    // Unregister first listener
    unregister1();
    
    // Trigger more events
    model.setUserInput(0, 1, 3, 'Test3');
    
    await setTimeout(0);
    
    assert.strictEqual(events1.length, 2, 'First listener should receive 2 events');
    assert.strictEqual(events2.length, 3, 'Second listener should receive 2 events');
    
    // Call the second unregister too
    unregister2();
});

test('fromICalcBytes loads model', () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    model.setUserInput(0, 1, 1, '42');
    const bytes = model.saveToICalc();
    const m2 = Model.fromICalcBytes(bytes);
    assert.strictEqual(m2.getCellContent(0, 1, 1), '42');
});

test('fromXLSXBytes loads model', () => {
    const model = new Model('Workbook1', 'en', 'UTC');
    model.setUserInput(0, 1, 1, '5');
    const bytes = model.saveToXLSX();
    const m2 = Model.fromXLSXBytes(bytes, 'Workbook1', 'en', 'UTC');
    assert.strictEqual(m2.getCellContent(0, 1, 1), '5');
});

test('roundtrip via xlsx bytes', () => {
    const m1 = new Model('Workbook1', 'en', 'UTC');
    m1.setUserInput(0, 1, 1, '7');
    m1.setUserInput(0, 1, 2, '=A1*3');
    const bytes = m1.saveToXLSX();
    const m2 = Model.fromXLSXBytes(bytes, 'Workbook1', 'en', 'UTC');
    m2.evaluate();
    assert.strictEqual(m2.getFormattedCellValue(0, 1, 2), '21');
});

test('roundtrip via icalc bytes', () => {
    const m1 = new Model('Workbook1', 'en', 'UTC');
    m1.setUserInput(0, 1, 1, '9');
    m1.setUserInput(0, 1, 2, '=A1*4');
    const bytes = m1.saveToICalc();
    const m2 = Model.fromICalcBytes(bytes);
    m2.evaluate();
    assert.strictEqual(m2.getFormattedCellValue(0, 1, 2), '36');
});
