#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_fn_avedev_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=AVEDEV()");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#ERROR!");
}

#[test]
fn test_fn_avedev_minimal() {
    let mut model = new_empty_model();
    model._set("B1", "1");
    model._set("B2", "2");
    model._set("B3", "3");
    model._set("B4", "'2");
    model._set("B6", "true");
    model._set("A1", "=AVEDEV(B1:B6)");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"0.666666667");
}

#[test]
fn test_fn_avedev_mathematical_validation() {
    let mut model = new_empty_model();
    // Test with values 2, 4, 6, 8
    // Mean = 5, deviations: |2-5|=3, |4-5|=1, |6-5|=1, |8-5|=3
    // Average deviation = (3+1+1+3)/4 = 2
    model._set("B1", "2");
    model._set("B2", "4");
    model._set("B3", "6");
    model._set("B4", "8");
    model._set("A1", "=AVEDEV(B1:B4)");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"2");
}

#[test]
fn test_fn_avedev_single_value() {
    let mut model = new_empty_model();
    model._set("B1", "10");
    model._set("A1", "=AVEDEV(B1)");
    model.evaluate();

    // Single value has zero deviation from its own mean
    assert_eq!(model._get_text("A1"), *"0");
}
