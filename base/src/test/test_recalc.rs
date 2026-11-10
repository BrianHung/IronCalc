#![allow(clippy::unwrap_used)]

use crate::recalc::{Everything, Ran, RecalcMode};
use crate::test::util::new_empty_model;
use crate::Model;

fn incremental_model<'a>() -> Model<'a> {
    new_empty_model().with_recalc_mode(RecalcMode::Incremental)
}

/// Whether the formula at `cell` ran in the last evaluation.
fn ran(model: &Model, cell: &str) -> bool {
    let reference = model._parse_reference(cell);
    model
        .evaluation
        .cells
        .contains_key(&(reference.sheet, reference.row, reference.column))
}

#[test]
fn an_edit_runs_only_the_formulas_that_read_it() {
    let mut model = incremental_model();
    model._set("A1", "1");
    model._set("B1", "=A1*2");
    model._set("C1", "=B1+1");
    model._set("D1", "=7*3");
    model.evaluate();

    model._set("A1", "5");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "10");
    assert_eq!(model._get_text("C1"), "11");
    assert!(ran(&model, "B1") && ran(&model, "C1"));
    assert!(!ran(&model, "D1"));
}

#[test]
fn a_whole_column_sees_a_write_beyond_the_used_area() {
    let mut model = incremental_model();
    model._set("A1", "1");
    model._set("B1", "=SUM(A:A)");
    model._set("D1", "=1+1");
    model.evaluate();

    model._set("A100", "5");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "6");
    assert!(!ran(&model, "D1"));
}

#[test]
fn offset_depends_on_where_it_points_now() {
    let mut model = incremental_model();
    model._set("A1", "1");
    model._set("A2", "10");
    model._set("B1", "2");
    model._set("C1", "=OFFSET(A1,B1-1,0)");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "10");

    model._set("A2", "20");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "20");

    model._set("B1", "1");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "1");

    // A2 is no longer read.
    model._set("A2", "30");
    model.evaluate();
    assert!(!ran(&model, "C1"));
}

#[test]
fn the_clock_and_random_numbers_are_read_every_time() {
    let mut model = incremental_model();
    model._set("A1", "=RAND()");
    model._set("B1", "=1+1");
    model.evaluate();

    model._set("C5", "3");
    model.evaluate();
    assert!(ran(&model, "A1"));
    assert!(!ran(&model, "B1"));
}

#[test]
fn a_formula_replaced_by_a_value_reads_nothing() {
    let mut model = incremental_model();
    model._set("A1", "1");
    model._set("B1", "=A1");
    model._set("C1", "=B1+1");
    model.evaluate();

    model._set("B1", "5");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "6");

    model._set("A1", "9");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "6");
    assert!(!ran(&model, "C1"));
}

#[test]
fn hiding_a_row_runs_what_reads_hidden_rows() {
    let mut model = incremental_model();
    model._set("A1", "1");
    model._set("A2", "2");
    model._set("A3", "4");
    model._set("B1", "=SUBTOTAL(109,A1:A3)");
    model._set("D1", "=1+1");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "7");

    model.set_row_hidden(0, 2, true).unwrap();
    model.evaluate();
    assert_eq!(model._get_text("B1"), "5");
    assert!(!ran(&model, "D1"));
}

#[test]
fn inserting_rows_runs_what_the_rows_cut_through() {
    let mut model = incremental_model();
    for row in 1..=10 {
        model._set(&format!("A{row}"), &row.to_string());
    }
    model._set("B1", "=A1+1");
    model._set("C1", "=SUM(A1:A10)");
    model._set("D1", "=ROW(A7)");
    model._set("E1", "=OFFSET(A1,6,0)");
    model._set("B9", "=A9*2");
    model._set("F1", "=SUM(A1:A3)");
    model.evaluate();
    assert_eq!(model._get_text("E1"), "7");

    model.insert_rows(0, 5, 1).unwrap();
    model.evaluate();
    assert_eq!(model.recalc.ran, Some(Ran::Only(3)));
    assert_eq!(model._get_formula("C1"), "=SUM(A1:A11)");
    assert_eq!(model._get_text("C1"), "55");
    assert_eq!(model._get_text("D1"), "8");
    assert_eq!(model._get_text("E1"), "6");
    assert_eq!(model._get_text("B10"), "18");
    // Neither what is above the new row nor what moved along with what it
    // reads is evaluated again.
    assert!(!ran(&model, "B1") && !ran(&model, "F1") && !ran(&model, "B10"));

    // What moved is still followed where it is now.
    model._set("A10", "100");
    model.evaluate();
    assert_eq!(model._get_text("B10"), "200");
    assert_eq!(model._get_text("C1"), "146");
    assert!(!ran(&model, "B1"));
}

#[test]
fn deleting_rows_runs_what_read_them() {
    let mut model = incremental_model();
    for row in 1..=10 {
        model._set(&format!("A{row}"), &row.to_string());
    }
    model._set("B1", "=A5");
    model._set("C1", "=SUM(A:A)");
    model._set("D1", "=A2+1");
    model._set("B9", "=A9*2");
    model.evaluate();

    model.delete_rows(0, 5, 1).unwrap();
    model.evaluate();
    assert_eq!(model._get_text("B1"), "#REF!");
    assert_eq!(model._get_text("C1"), "50");
    assert_eq!(model._get_text("B8"), "18");
    assert!(!ran(&model, "D1") && !ran(&model, "B8"));

    model._set("A8", "0");
    model.evaluate();
    assert_eq!(model._get_text("B8"), "0");
    assert_eq!(model._get_text("C1"), "41");
}

#[test]
fn columns_move_like_rows() {
    let mut model = incremental_model();
    model._set("A1", "1");
    model._set("B1", "2");
    model._set("C1", "=A1+B1");
    model._set("E1", "=C1*10");
    model._set("A3", "=A1+1");
    model._set("C3", "=COLUMN()");
    model.evaluate();

    model.insert_columns(0, 2, 2).unwrap();
    model.evaluate();
    assert_eq!(model._get_formula("E1"), "=A1+D1");
    assert_eq!(model._get_text("E1"), "3");
    assert_eq!(model._get_text("G1"), "30");
    assert_eq!(model._get_text("E3"), "5");
    assert!(!ran(&model, "A3"));

    model.delete_columns(0, 2, 2).unwrap();
    model.evaluate();
    assert_eq!(model._get_text("C1"), "3");
    assert_eq!(model._get_text("E1"), "30");
    assert_eq!(model._get_text("C3"), "3");
}

#[test]
fn an_array_in_the_scope_spills_again() {
    let mut model = incremental_model();
    model._set("A1", "2");
    model._set("B1", "=SEQUENCE(A1)");
    model._set("C1", "=SUM(B1:B10)");
    model._set("D1", "=B2*10");
    model._set("F1", "=1+1");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "3");

    model._set("A1", "4");
    model.evaluate();
    assert!(matches!(model.recalc.ran, Some(Ran::Only(_))));
    assert_eq!(model._get_text("B4"), "4");
    assert_eq!(model._get_text("C1"), "10");
    assert_eq!(model._get_text("D1"), "20");
    assert!(!ran(&model, "F1"));

    model._set("A1", "1");
    model.evaluate();
    assert_eq!(model._get_text("B2"), "");
    assert_eq!(model._get_text("C1"), "1");
    assert_eq!(model._get_text("D1"), "0");
}

#[test]
fn an_array_that_grows_reaches_what_read_the_cells_it_takes() {
    let mut model = incremental_model();
    model._set("A1", "2");
    model._set("B1", "=SEQUENCE(A1)");
    // Reads B4 while it is empty, and nothing else of the array.
    model._set("D1", "=IF(ISBLANK(B4),\"free\",\"taken\")");
    model._set("E1", "=D1&\"!\"");
    model.evaluate();
    assert_eq!(model._get_text("E1"), "free!");

    model._set("A1", "5");
    model.evaluate();
    assert_eq!(model._get_text("E1"), "taken!");

    model._set("A1", "3");
    model.evaluate();
    assert_eq!(model._get_text("E1"), "free!");
}

#[test]
fn unmerging_what_blocked_a_spill_lets_it_spill() {
    let mut model = incremental_model();
    model._set("A1", "=SEQUENCE(3)");
    model._set("C1", "=1+1");
    model
        .merge_cells(&crate::expressions::types::Area {
            sheet: 0,
            row: 2,
            column: 1,
            width: 2,
            height: 2,
        })
        .unwrap();
    model.evaluate();
    assert_eq!(model._get_text("A1"), "#SPILL!");

    model
        .unmerge_cells(&crate::expressions::types::Area {
            sheet: 0,
            row: 2,
            column: 1,
            width: 2,
            height: 2,
        })
        .unwrap();
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1");
    assert_eq!(model._get_text("A3"), "3");
    assert!(!ran(&model, "C1"));
}

#[test]
fn only_what_formulas_mean_changing_runs_everything() {
    let mut model = incremental_model();
    model._set("A1", "1");
    model._set("B1", "=A1+1");
    model.evaluate();
    assert_eq!(
        model.recalc.ran,
        Some(Ran::Everything(Everything::FirstEvaluation))
    );

    model._set("A1", "2");
    model.evaluate();
    assert_eq!(model.recalc.ran, Some(Ran::Only(1)));

    // Adding a sheet evaluates.
    model.new_sheet();
    assert_eq!(
        model.recalc.ran,
        Some(Ran::Everything(Everything::WorkbookChanged))
    );

    model.insert_rows(0, 1, 1).unwrap();
    model.set_row_hidden(0, 5, true).unwrap();
    model._set("A9", "3");
    model.evaluate();
    assert_eq!(model.recalc.ran, Some(Ran::Only(0)));
}

#[test]
fn clearing_what_blocked_a_spill_lets_it_spill() {
    let mut model = incremental_model();
    model._set("A1", "=SEQUENCE(3)");
    model.evaluate();

    model._set("A2", "x");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "#SPILL!");

    model._set("A2", "");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1");
    assert_eq!(model._get_text("A3"), "3");
}

#[test]
fn a_cycle_outside_the_edit_keeps_its_error() {
    let mut model = incremental_model();
    model._set("A1", "=B1");
    model._set("B1", "=A1");
    model._set("C1", "=D1*2");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "#CIRC!");

    model._set("D1", "4");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "8");
    assert_eq!(model._get_text("A1"), "#CIRC!");
}

#[test]
fn a_reference_worked_out_by_a_function_does_not_follow_the_rows() {
    let mut model = incremental_model();
    model._set("A1", "7");
    // Above the first row there is nothing: an error, not a reference.
    model._set("C1", "=IFERROR(OFFSET(A1,-1,0),\"caught\")");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "caught");

    // The formula moves along with the cell it names, and reads another.
    model.insert_rows(0, 1, 1).unwrap();
    model.evaluate();
    assert_eq!(
        model._get_formula("C2"),
        "=IFERROR(OFFSET(A2,-1,0),\"caught\")"
    );
    assert_eq!(model._get_text("C2"), "0");
}

#[test]
fn a_name_does_not_follow_the_rows() {
    let mut model = incremental_model();
    model._set("A1", "1");
    model._set("A2", "7");
    model
        .new_defined_name(crate::model::DefinedNameEntry {
            name: "Seven".to_string(),
            scope: None,
            formula: "Sheet1!$A$2".to_string(),
            ..Default::default()
        })
        .unwrap();
    model._set("C5", "=Seven+1");
    model.evaluate();
    assert_eq!(model._get_text("C5"), "8");

    let mut full = new_empty_model().with_recalc_mode(RecalcMode::Full);
    full._set("A1", "1");
    full._set("A2", "7");
    full.new_defined_name(crate::model::DefinedNameEntry {
        name: "Seven".to_string(),
        scope: None,
        formula: "Sheet1!$A$2".to_string(),
        ..Default::default()
    })
    .unwrap();
    full._set("C5", "=Seven+1");
    full.evaluate();

    model.insert_rows(0, 1, 1).unwrap();
    model.evaluate();
    full.insert_rows(0, 1, 1).unwrap();
    full.evaluate();
    assert_eq!(model._get_text("C6"), full._get_text("C6"));
}
