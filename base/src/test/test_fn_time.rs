#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn time_functions() {
    let mut model = new_empty_model();
    model._set("A1", "=TIME(14,30,0)");
    model._set("A2", "=TIME(27,0,0)");
    model._set("A3", "=TIMEVALUE(\"14:30\")");
    model._set("B1", "=HOUR(A1)");
    model._set("B2", "=MINUTE(A1)");
    model._set("B3", "=SECOND(A1)");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"0.604166667");
    assert_eq!(model._get_text("A2"), *"0.125");
    assert_eq!(model._get_text("A3"), *"0.604166667");
    assert_eq!(model._get_text("B1"), *"14");
    assert_eq!(model._get_text("B2"), *"30");
    assert_eq!(model._get_text("B3"), *"0");
}
