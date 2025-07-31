#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn fact_basic() {
    let mut model = new_empty_model();
    model._set("A1", "=FACT(3)");
    model._set("A2", "=FACT(-1)");
    model._set("A3", "=FACT()");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"6");
    assert_eq!(model._get_text("A2"), *"#NUM!");
    assert_eq!(model._get_text("A3"), *"#ERROR!");
}
