#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_fn_percentile() {
    let mut model = new_empty_model();
    for i in 0..5 {
        model._set(&format!("B{}", i + 1), &(i + 1).to_string());
    }
    model._set("A1", "=PERCENTILE.INC(B1:B5,0.4)");
    model._set("A2", "=PERCENTILE.EXC(B1:B5,0.4)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"2.6");
    assert_eq!(model._get_text("A2"), *"2.4");
}

#[test]
fn test_fn_percentrank() {
    let mut model = new_empty_model();
    for i in 0..5 {
        model._set(&format!("B{}", i + 1), &(i + 1).to_string());
    }
    model._set("A1", "=PERCENTRANK.INC(B1:B5,3.5)");
    model._set("A2", "=PERCENTRANK.EXC(B1:B5,3.5)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"0.625");
    assert_eq!(model._get_text("A2"), *"0.583");
}
