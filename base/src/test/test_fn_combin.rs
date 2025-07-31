#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn combin_family() {
    let mut model = new_empty_model();
    model._set("A1", "=COMBIN(10,3)");
    model._set("A2", "=COMBINA(4,2)");
    model._set("A3", "=PERMUT(10,3)");
    model._set("A4", "=PERMUTATIONA(10,3)");
    model._set("A5", "=COMBIN(1)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"120");
    assert_eq!(model._get_text("A2"), *"10");
    assert_eq!(model._get_text("A3"), *"720");
    assert_eq!(model._get_text("A4"), *"1000");
    assert_eq!(model._get_text("A5"), *"#ERROR!");
}
