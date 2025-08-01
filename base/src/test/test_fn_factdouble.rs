#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=FACTDOUBLE()");
    model._set("A2", "=FACTDOUBLE(1,2)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#ERROR!");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
}

#[test]
fn values() {
    let mut model = new_empty_model();
    model._set("A1", "=FACTDOUBLE(8)");
    model._set("A2", "=FACTDOUBLE(7)");
    model._set("A3", "=FACTDOUBLE(0)");
    model._set("A4", "=FACTDOUBLE(-1)");
    model._set("A5", "=FACTDOUBLE(5.9)");
    model._set("A6", "=FACTDOUBLE(-2)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"384");
    assert_eq!(model._get_text("A2"), *"105");
    assert_eq!(model._get_text("A3"), *"1");
    assert_eq!(model._get_text("A4"), *"1");
    assert_eq!(model._get_text("A5"), *"15");
    assert_eq!(model._get_text("A6"), *"#NUM!");
}
