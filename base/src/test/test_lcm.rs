#![allow(clippy::unwrap_used)]
use crate::test::util::new_empty_model;

#[test]
fn test_fn_lcm_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=LCM()");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#ERROR!");
}

#[test]
fn test_fn_lcm_basic_functionality() {
    let mut model = new_empty_model();

    // Single argument
    model._set("A1", "=LCM(12)");
    model._set("A2", "=LCM(1)");

    // Multiple arguments
    model._set("A3", "=LCM(25,40)");
    model._set("A4", "=LCM(4,6,8)");
    model._set("A5", "=LCM(12,15,20)");

    // With zeros (LCM with any zero = 0)
    model._set("A6", "=LCM(0)");
    model._set("A7", "=LCM(0,12)");
    model._set("A8", "=LCM(12,0)");
    model._set("A9", "=LCM(10,0,20)");

    // Decimal inputs (should truncate)
    model._set("A10", "=LCM(4.7,6.3)");

    // Edge cases
    model._set("A11", "=LCM(1,1)");
    model._set("A12", "=LCM(1,2,3,4,5)");

    // Large numbers (simpler ones)
    model._set("A13", "=LCM(100,150)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"12");
    assert_eq!(model._get_text("A2"), *"1");
    assert_eq!(model._get_text("A3"), *"200");
    assert_eq!(model._get_text("A4"), *"24");
    assert_eq!(model._get_text("A5"), *"60");
    assert_eq!(model._get_text("A6"), *"0");
    assert_eq!(model._get_text("A7"), *"0");
    assert_eq!(model._get_text("A8"), *"0");
    assert_eq!(model._get_text("A9"), *"0");
    assert_eq!(model._get_text("A10"), *"12");
    assert_eq!(model._get_text("A11"), *"1");
    assert_eq!(model._get_text("A12"), *"60");
    assert_eq!(model._get_text("A13"), *"300");
}

#[test]
fn test_fn_lcm_error_cases() {
    let mut model = new_empty_model();

    // Negative numbers
    model._set("A1", "=LCM(-5)");
    model._set("A2", "=LCM(12,-8)");

    // Non-finite values
    model._set("B1", "=1/0"); // Infinity
    model._set("B2", "=0/0"); // NaN
    model._set("A3", "=LCM(B1)");
    model._set("A4", "=LCM(B2)");
    model._set("A5", "=LCM(12,B1)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#NUM!");
    assert_eq!(model._get_text("A2"), *"#NUM!");
    assert_eq!(model._get_text("A3"), *"#DIV/0!");
    assert_eq!(model._get_text("A4"), *"#DIV/0!");
    assert_eq!(model._get_text("A5"), *"#DIV/0!");
}

#[test]
fn test_fn_lcm_ranges_and_mixed() {
    let mut model = new_empty_model();

    // Range inputs
    model._set("B1", "4");
    model._set("B2", "6");
    model._set("B3", "8");
    model._set("A1", "=LCM(B1:B3)");

    // Mixed inputs (numbers, text, empty cells)
    model._set("C1", "4");
    model._set("C2", "text"); // Should be ignored
    model._set("C3", "6");
    // C4 is empty, should be ignored
    model._set("C5", "8");
    model._set("A2", "=LCM(C1:C5)");

    // Zero in range (should return 0)
    model._set("D1", "4");
    model._set("D2", "0");
    model._set("D3", "6");
    model._set("A3", "=LCM(D1:D3)");

    // No valid numbers case
    model._set("E1", "text");
    model._set("A4", "=LCM(E1,E2)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"24");
    assert_eq!(model._get_text("A2"), *"24");
    assert_eq!(model._get_text("A3"), *"0");
    assert_eq!(model._get_text("A4"), *"0");
}
