# IronCalc Web bindings

This package contains web bindings for IronCalc. It exposes the engine and helper functions to import or export workbooks as XLSX or IronCalc (icalc) byte arrays. The built-in XLSX support focuses on core spreadsheet features like cell values, formulas, and styling.


## Usage

In your project

```
npm install @ironcalc/wasm
```

And then in your TypeScript

```TypeScript
import init, { Model } from "@ironcalc/wasm";

await init();

function compute() {
    const model = new Model('en', 'UTC');
    
    model.setUserInput(0, 1, 1, "23");
    model.setUserInput(0, 1, 2, "=A1*3+1");
    
    const result = model.getFormattedCellValue(0, 1, 2);
    
    console.log("Result: ", result);
}

compute();
```

To listen to model changes you can subscribe to diff events:

```TypeScript
import init, { Model } from "@ironcalc/wasm";

await init();

const model = new Model("Sheet1", "en", "UTC");

model.onDiffs(() => {
    // React to diff list updates
    redraw();
});

model.setUserInput(0, 1, 1, "=1+1");
```
### Importing and exporting bytes

The `Model` class provides helpers to load or save workbooks as raw byte arrays.

```ts
// create a new workbook and export as XLSX bytes
const model = new Model('Workbook1', 'en', 'UTC');
model.setUserInput(0, 1, 1, '42');
const xlsxBytes = model.saveToXlsx();

// load from those bytes
const roundTripped = Model.fromXlsxBytes(xlsxBytes, 'Workbook1', 'en', 'UTC');

// same helpers exist for IronCalc's internal format
const icalcBytes = model.saveToIcalc();
const restored = Model.fromIcalcBytes(icalcBytes);
```


To listen to cells that change during evaluation (formulas that recalculate):

```TypeScript
import init, { Model } from "@ironcalc/wasm";

await init();

const model = new Model("Sheet1", "en", "UTC");

model.onCellsEvaluated((cellReferences) => {
    // cellReferences is an array of {sheet, row, column} objects
    // that represent cells that were recalculated during evaluation
    cellReferences.forEach(cell => {
        console.log(`Cell ${cell.sheet}:${cell.row}:${cell.column} was evaluated`);
    });
});

// Setting a formula will trigger evaluation
model.setUserInput(0, 1, 1, "=SUM(A2:A5)");
model.setUserInput(0, 2, 1, "10");  // This will trigger re-evaluation of A1
```
