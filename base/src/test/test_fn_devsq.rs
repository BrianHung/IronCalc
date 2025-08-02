#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_fn_devsq_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=DEVSQ()");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#ERROR!");
}

#[test]
fn test_fn_devsq_minimal() {
    let mut model = new_empty_model();
    // Data from mathematical example: 4,5,8,7,11,4,3 -> result 48
    model._set("B1", "4");
    model._set("B2", "5");
    model._set("B3", "8");
    model._set("B4", "7");
    model._set("B5", "11");
    model._set("B6", "4");
    model._set("B7", "3");
    model._set("A1", "=DEVSQ(B1:B7)");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"48");
}

#[test]
fn test_fn_devsq_simple_validation() {
    let mut model = new_empty_model();
    // Test with values 1, 3, 5
    // Mean = 3, deviations: (1-3)²=4, (3-3)²=0, (5-3)²=4
    // Sum of squared deviations = 4+0+4 = 8
    model._set("B1", "1");
    model._set("B2", "3");
    model._set("B3", "5");
    model._set("A1", "=DEVSQ(B1:B3)");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"8");
}

#[test]
fn test_fn_devsq_single_value() {
    let mut model = new_empty_model();
    model._set("B1", "10");
    model._set("A1", "=DEVSQ(B1)");
    model.evaluate();

    // Single value has zero squared deviation from its own mean
    assert_eq!(model._get_text("A1"), *"0");
}
