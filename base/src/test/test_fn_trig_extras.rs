#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn cot_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=COT(PI()/4)");
    model._set("A2", "=COT()");
    model._set("A3", "=COT(1,2)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"1");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
    assert_eq!(model._get_text("A3"), *"#ERROR!");
}

#[test]
fn coth_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=COTH(1)");
    model._set("A2", "=COTH()");
    model._set("A3", "=COTH(1,2)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"1.313035285");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
    assert_eq!(model._get_text("A3"), *"#ERROR!");
}

#[test]
fn acot_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=ACOT(2)");
    model._set("A2", "=ACOT()");
    model._set("A3", "=ACOT(1,2)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"0.463647609");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
    assert_eq!(model._get_text("A3"), *"#ERROR!");
}

#[test]
fn acoth_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=ACOTH(2)");
    model._set("A2", "=ACOTH()");
    model._set("A3", "=ACOTH(2,2)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"0.549306144");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
    assert_eq!(model._get_text("A3"), *"#ERROR!");
}

#[test]
fn csc_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=CSC(PI()/2)");
    model._set("A2", "=CSC()");
    model._set("A3", "=CSC(1,2)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"1");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
    assert_eq!(model._get_text("A3"), *"#ERROR!");
}

#[test]
fn csch_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=CSCH(1)");
    model._set("A2", "=CSCH()");
    model._set("A3", "=CSCH(1,2)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"0.850918128");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
    assert_eq!(model._get_text("A3"), *"#ERROR!");
}

#[test]
fn sec_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=SEC(0)");
    model._set("A2", "=SEC()");
    model._set("A3", "=SEC(1,2)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"1");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
    assert_eq!(model._get_text("A3"), *"#ERROR!");
}

#[test]
fn sech_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=SECH(1)");
    model._set("A2", "=SECH()");
    model._set("A3", "=SECH(1,2)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"0.648054274");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
    assert_eq!(model._get_text("A3"), *"#ERROR!");
}

