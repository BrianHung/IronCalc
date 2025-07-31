#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn combin_comprehensive() {
    let mut model = new_empty_model();

    model._set("A1", "=COMBIN(5,0)");
    model._set("A2", "=COMBIN(5,5)");
    model._set("A3", "=COMBIN(10,3)");
    model._set("A4", "=COMBIN(7,1)");
    model._set("A5", "=COMBIN(10.9,3.2)");
    model._set("A6", "=COMBIN(3,4)");
    model._set("A7", "=COMBIN(-1,2)");
    model._set("A8", "=COMBIN()");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"1");
    assert_eq!(model._get_text("A2"), *"1");
    assert_eq!(model._get_text("A3"), *"120");
    assert_eq!(model._get_text("A4"), *"7");
    assert_eq!(model._get_text("A5"), *"120");
    assert_eq!(model._get_text("A6"), *"#NUM!");
    assert_eq!(model._get_text("A7"), *"#NUM!");
    assert_eq!(model._get_text("A8"), *"#ERROR!");
}

#[test]
fn combin_mathematical_properties() {
    let mut model = new_empty_model();

    model._set("A1", "=COMBIN(8,3)");
    model._set("A2", "=COMBIN(8,5)");
    model._set("A3", "=COMBIN(5,2)");
    model._set("A4", "=COMBIN(4,1)+COMBIN(4,2)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), model._get_text("A2"));
    assert_eq!(model._get_text("A3"), model._get_text("A4"));
}
