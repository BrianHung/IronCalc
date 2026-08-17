#![allow(clippy::unwrap_used)]

//! Structural edits displace every formula's references. These tests exercise
//! the in-place AST transform (relative, absolute and range references), its
//! fall back to the string path when a reference is deleted (#REF!), and that a
//! serialize round trip preserves the displaced formulas.

use crate::constants::LAST_ROW;
use crate::model::Model;
use crate::test::util::new_empty_model;

#[test]
fn insert_rows_shifts_relative_and_absolute_references() {
    let mut model = new_empty_model();
    model._set("A1", "10");
    model._set("A2", "20");
    model._set("B1", "=A1+A2");
    model._set("C1", "=$A$1+$A$2");
    model._set("D1", "=SUM(A1:A2)");
    model.evaluate();

    model.insert_rows(0, 1, 2).unwrap();
    model.evaluate();

    assert_eq!(model._get_formula("B3"), "=A3+A4");
    assert_eq!(model._get_formula("C3"), "=$A$3+$A$4");
    assert_eq!(model._get_formula("D3"), "=SUM(A3:A4)");
    assert_eq!(model._get_text("B3"), "30");
    assert_eq!(model._get_text("C3"), "30");
    assert_eq!(model._get_text("D3"), "30");
}

#[test]
fn insert_columns_shifts_references() {
    let mut model = new_empty_model();
    model._set("A1", "10");
    model._set("B1", "20");
    model._set("C1", "=A1+B1");
    model.evaluate();

    model.insert_columns(0, 1, 1).unwrap();
    model.evaluate();

    // A1,B1 move to B1,C1; C1 formula moves to D1 and its refs shift.
    assert_eq!(model._get_formula("D1"), "=B1+C1");
    assert_eq!(model._get_text("D1"), "30");
}

#[test]
fn delete_rows_that_delete_a_reference_yield_ref_error() {
    let mut model = new_empty_model();
    model._set("A5", "5");
    model._set("B1", "=A5");
    model.evaluate();

    // Delete rows 4..6, removing A5. B1 stays put; its reference is gone.
    model.delete_rows(0, 4, 3).unwrap();
    model.evaluate();

    assert!(model._get_formula("B1").contains("#REF!"));
    assert_eq!(model._get_text("B1"), "#REF!");
}

#[test]
fn insert_rows_pushing_reference_off_the_bottom_yields_ref_error() {
    let mut model = new_empty_model();
    // B1 stays put (row 1 < insertion point) but its reference to the last row
    // is pushed one row past the sheet, so it must become #REF!.
    model._set("B1", &format!("=A{LAST_ROW}"));
    model.evaluate();

    model.insert_rows(0, 2, 1).unwrap();
    model.evaluate();

    assert!(model._get_formula("B1").contains("#REF!"));
    assert_eq!(model._get_text("B1"), "#REF!");
}

#[test]
fn delete_rows_shifts_references_below() {
    let mut model = new_empty_model();
    model._set("A10", "7");
    model._set("B1", "=A10");
    model.evaluate();

    model.delete_rows(0, 2, 3).unwrap();
    model.evaluate();

    // A10 moves up to A7; the reference follows.
    assert_eq!(model._get_formula("B1"), "=A7");
    assert_eq!(model._get_text("B1"), "7");
}

#[test]
fn serialize_round_trip_after_structural_edit() {
    let mut model = new_empty_model();
    model._set("A1", "1");
    model._set("A2", "2");
    model._set("B1", "=A1+A2");
    model._set("C1", "=SUM(A1:A2)*$A$1");
    model.evaluate();

    model.insert_rows(0, 1, 3).unwrap();
    model.evaluate();

    let bytes = model.to_bytes();
    let mut reloaded = Model::from_bytes(&bytes, "en").unwrap();
    reloaded.evaluate();

    for cell in ["B4", "C4"] {
        assert_eq!(model._get_formula(cell), reloaded._get_formula(cell));
        assert_eq!(model._get_text(cell), reloaded._get_text(cell));
    }
    assert_eq!(reloaded._get_formula("B4"), "=A4+A5");
    assert_eq!(reloaded._get_formula("C4"), "=SUM(A4:A5)*$A$4");
}
