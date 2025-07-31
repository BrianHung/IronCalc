#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn permut_comprehensive() {
    let mut model = new_empty_model();

    model._set("A1", "=PERMUT(5,0)");
    model._set("A2", "=PERMUT(4,4)");
    model._set("A3", "=PERMUT(8,3)");
    model._set("A4", "=PERMUT(6,1)");
    model._set("A5", "=PERMUT(8.7,2.3)");
    model._set("A6", "=PERMUT(3,5)");
    model._set("A7", "=PERMUT(-1,2)");
    model._set("A8", "=PERMUT()");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"1");
    assert_eq!(model._get_text("A2"), *"24");
    assert_eq!(model._get_text("A3"), *"336");
    assert_eq!(model._get_text("A4"), *"6");
    assert_eq!(model._get_text("A5"), *"56");
    assert_eq!(model._get_text("A6"), *"#NUM!");
    assert_eq!(model._get_text("A7"), *"#NUM!");
    assert_eq!(model._get_text("A8"), *"#ERROR!");
}

#[test]
fn permut_mathematical_relationships() {
    let mut model = new_empty_model();

    model._set("A1", "=PERMUT(6,3)");
    model._set("A2", "=COMBIN(6,3)*FACT(3)");
    model._set("A3", "=PERMUT(5,5)");
    model._set("A4", "=FACT(5)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), model._get_text("A2"));
    assert_eq!(model._get_text("A3"), model._get_text("A4"));
}

#[test]
fn permut_overflow() {
    let mut model = new_empty_model();

    model._set("A1", "=PERMUT(100, 50)");
    model._set("A2", "=PERMUT(1000, 500)");
    model._set("A3", "=PERMUT(200, 100)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"3.06852E+93");
    assert_eq!(model._get_text("A2"), *"#NUM!");
    assert_eq!(model._get_text("A3"), *"8.45055E+216");
}
