#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_fn_stdev_var_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=STDEVA()");
    model._set("A2", "=STDEVPA()");
    model._set("A3", "=VARA()");
    model._set("A4", "=VARPA()");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#ERROR!");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
    assert_eq!(model._get_text("A3"), *"#ERROR!");
    assert_eq!(model._get_text("A4"), *"#ERROR!");
}

#[test]
fn test_fn_stdev_var_minimal() {
    let mut model = new_empty_model();
    model._set("B1", "1");
    model._set("B2", "2");
    model._set("B3", "3");
    model._set("B4", "'2");
    model._set("B6", "true");
    model._set("A1", "=STDEVA(B1:B6)");
    model._set("A2", "=STDEVPA(B1:B6)");
    model._set("A3", "=VARA(B1:B6)");
    model._set("A4", "=VARPA(B1:B6)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"1.140175425");
    assert_eq!(model._get_text("A2"), *"1.019803903");
    assert_eq!(model._get_text("A3"), *"1.3");
    assert_eq!(model._get_text("A4"), *"1.04");
}
