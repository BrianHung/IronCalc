#![allow(clippy::unwrap_used)]

use crate::test::util::{incremental_mode, new_empty_model};

#[test]
fn simple_colum() {
    let mut model = new_empty_model();
    // We populate cells A1 to A3
    model._set("A1", "1");
    model._set("A2", "2");
    model._set("A3", "3");

    model._set("C2", "=@A1:A3");

    model.evaluate();

    assert_eq!(model._get_text("C2"), "2".to_string());
}

#[test]
fn return_of_array_spills() {
    let mut model = new_empty_model();
    // We populate cells A1 to A3
    model._set("A1", "1");
    model._set("A2", "2");
    model._set("A3", "3");

    // With dynamic arrays, =A1:A3 spills downward from C2
    model._set("C2", "=A1:A3");
    model._set("D2", "=SUM(SIN(A:A)");

    model.evaluate();

    assert_eq!(model._get_text("C2"), "1".to_string());
    assert_eq!(model._get_text("C3"), "2".to_string());
    assert_eq!(model._get_text("C4"), "3".to_string());
    assert_eq!(model._get_text("D2"), "1.89188842".to_string());
}

#[test]
fn concat() {
    let mut model = new_empty_model();
    model._set("A1", "=CONCAT(@B1:B3)");
    model._set("A2", "=CONCAT(B1:B3)");
    model._set("B1", "Hello");
    model._set("B2", " ");
    model._set("B3", "world!");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"Hello");
    assert_eq!(model._get_text("A2"), *"Hello world!");
}

#[test]
fn scalar_context_unwraps_1x1_array_from_offset() {
    // When a non-array formula produces a 1x1 array (e.g. via OFFSET), the
    // unwrapped scalar must be the value stored in the cell.
    let mut model = new_empty_model();
    model._set("B1", "10");
    model._set("B2", "20");
    model._set("B3", "30");

    model._set("A1", "=2 * IF(TRUE, OFFSET(B1, 2, 0), 0)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), "60".to_string());
}

#[test]
fn scalar_context_unwraps_1x1_array_from_offset_for_dependents() {
    // When a non-array formula produces a 1x1 array (e.g. via OFFSET), the
    // unwrapped scalar must be visible to dependents that are evaluated in the
    // same recalculation pass via `ReferenceKind -> evaluate_cell(...)`.
    let mut model = new_empty_model();
    model._set("B1", "10");
    model._set("B2", "20");
    model._set("B3", "30");

    model._set("A1", "=IF(TRUE, OFFSET(B1, 2, 0), 0)");
    model._set("C1", "=A1 + 1");
    model._set("D1", "=A1");

    model.evaluate();

    assert_eq!(model._get_text("A1"), "30".to_string());
    assert_eq!(model._get_text("C1"), "31".to_string());
    assert_eq!(model._get_text("D1"), "30".to_string());
}

/// Seeds 20/23: a formula written while a name is undefined parses as a scalar
/// `NamedVariable`. Once the name is defined as a *range*, the reparse must wrap
/// it in the implicit-intersection operator, or a 1xN array reaches a
/// scalar-context cell and trips the debug guard.
#[test]
fn name_defined_as_range_after_the_formula_stays_scalar() {
    let run = |mode: crate::RecalcMode| -> String {
        let mut model = new_empty_model().with_recalc_mode(mode);
        model._set("D6", "=NRANGE");
        model
            .new_defined_name("NRANGE", None, "Sheet1!$A$1:$A$8")
            .unwrap();
        model.evaluate();
        model._get_text("D6")
    };
    // Implicit intersection against A1:A8 from row 6 picks A6 (empty -> 0).
    assert_eq!(run(crate::RecalcMode::Full), "0");
    assert_eq!(run(crate::RecalcMode::Full), run(incremental_mode()));
}

/// The seed 23 variant: the formula references the name under its *old*
/// identity, and the rename retargets it to a range in one step. Implicit
/// intersection of E1:E5 from row 12 has no intersection, so `#VALUE!` is the
/// classic answer -- what must not happen is a panic or an array reaching D12.
#[test]
fn name_renamed_onto_a_range_stays_scalar() {
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Full);
    model._set("F12", "=NDATA");
    model
        .new_defined_name("NDATA", None, "Sheet1!$A$2")
        .unwrap();
    model
        .update_defined_name("NDATA", None, "NRANGE", None, "Sheet1!$E$1:$E$5")
        .unwrap();
    model.evaluate();
    assert_eq!(model._get_text("F12"), "#VALUE!");
}
