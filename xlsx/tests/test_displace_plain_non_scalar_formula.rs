#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

use ironcalc::import::load_from_xlsx;

// A cell shape only an import can produce. `set_user_input` runs static
// analysis and stores anything non-scalar as an `ArrayFormula`, so a *plain*
// formula cell whose AST analyzes non-scalar is unreachable through the public
// API. A file has no such gate: Excel stores `=LET(x,A6:A8,x)` and
// `={1,2,3}+A6` as ordinary `<f>` cells, and neither picks up an automatic
// implicit intersection on import (LET is exempt, an inline array is not
// intersected), so both land as plain `Cell::CellFormula`.
//
// The displacement below inserts a row *under* them, so the two cells never
// move — only their references shift. That matters: moving a cell rewrites it
// through the input path, which would promote it to an array formula first and
// hide the shape. Displacing it in place is the case worth pinning: the
// displaced AST is still non-scalar, so it has to be written back through the
// path that promotes it to a dynamic array. Storing it as a plain formula
// again leaves the cell holding a `#VALUE!` with nothing spilled below it.
#[test]
fn test_displacing_an_imported_plain_non_scalar_formula() {
    let mut model =
        load_from_xlsx("tests/plain_non_scalar_formula.xlsx", "en", "UTC", "en").unwrap();
    // Deliberately no evaluate here: evaluating would promote both cells to
    // array formulas before the displacement ever sees them.
    assert_eq!(
        model.get_cell_formula(0, 1, 3).unwrap(),
        Some("=LET(x,A6:A8,x)".to_string())
    );
    assert_eq!(
        model.get_cell_formula(0, 1, 5).unwrap(),
        Some("={1,2,3}+A6".to_string())
    );

    model.insert_rows(0, 5, 1).unwrap();
    model.evaluate();

    // Both stayed put, their references shifted, and both spill.
    assert_eq!(
        model.get_cell_formula(0, 1, 3).unwrap(),
        Some("=LET(x,A7:A9,x)".to_string())
    );
    assert_eq!(model.get_formatted_cell_value(0, 1, 3).unwrap(), "10");
    assert_eq!(model.get_formatted_cell_value(0, 2, 3).unwrap(), "20");
    assert_eq!(model.get_formatted_cell_value(0, 3, 3).unwrap(), "30");

    assert_eq!(
        model.get_cell_formula(0, 1, 5).unwrap(),
        Some("={1,2,3}+A7".to_string())
    );
    assert_eq!(model.get_formatted_cell_value(0, 1, 5).unwrap(), "11");
    assert_eq!(model.get_formatted_cell_value(0, 1, 6).unwrap(), "12");
    assert_eq!(model.get_formatted_cell_value(0, 1, 7).unwrap(), "13");
}
