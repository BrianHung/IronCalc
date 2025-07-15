#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn basic_address() {
    let mut model = new_empty_model();
    model._set("A1", "=ADDRESS(1,1)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"$A$1");
}

#[test]
fn address_with_sheet_and_r1c1() {
    let mut model = new_empty_model();
    model.new_sheet();
    model._set("A1", "=ADDRESS(4,3,2,FALSE,\"Sheet2\")");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"Sheet2!R4C[3]");
}

#[test]
fn address_invalid() {
    let mut model = new_empty_model();
    model._set("A1", "=ADDRESS(0,1)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#VALUE!");
}
