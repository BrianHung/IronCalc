#![allow(clippy::unwrap_used)]

use crate::recalc::RecalcMode;
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
fn hiding_a_row_is_evaluated_in_full() {
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
    assert!(ran(&model, "D1"));
}

#[test]
fn inserting_rows_is_evaluated_in_full() {
    let mut model = incremental_model();
    model._set("A1", "1");
    model._set("B5", "=A1+1");
    model._set("D1", "=1+1");
    model.evaluate();

    model.insert_rows(0, 2, 1).unwrap();
    model.evaluate();
    assert_eq!(model._get_text("B6"), "2");
    assert!(ran(&model, "D1"));
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
