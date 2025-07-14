#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn fn_isna_args_number() {
    let mut model = new_empty_model();
    model._set("A1", "=ISNA()");
    model._set("A2", "=ISNA(1, 2)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#ERROR!");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
}

#[test]
fn fn_isna() {
    let mut model = new_empty_model();
    model._set("A1", "#N/A");
    model._set("A2", "=1/0");
    model._set("A3", "42");

    model._set("B1", "=ISNA(A1)");
    model._set("B2", "=ISNA(A2)");
    model._set("B3", "=ISNA(A3)");

    model.evaluate();

    assert_eq!(model._get_text("B1"), *"TRUE");
    assert_eq!(model._get_text("B2"), *"FALSE");
    assert_eq!(model._get_text("B3"), *"FALSE");
}
