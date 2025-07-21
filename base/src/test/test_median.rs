#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_fn_median_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=MEDIAN()");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#ERROR!");
}

#[test]
fn test_fn_median_minimal() {
    let mut model = new_empty_model();
    model._set("B1", "1");
    model._set("B2", "2");
    model._set("B3", "3");
    model._set("B4", "'2");
    // B5 empty
    model._set("B6", "true");
    model._set("A1", "=MEDIAN(B1:B6)");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"2");
}

#[test]
fn test_fn_median_empty_values_error() {
    let mut model = new_empty_model();
    // Test with only non-numeric values (should return #DIV/0! error, not 0)
    model._set("B1", "\"text\"");
    model._set("B2", "\"more text\"");
    model._set("B3", "");  // empty cell
    model._set("A1", "=MEDIAN(B1:B3)");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#DIV/0!");
}

#[test]
fn test_fn_median_with_error_values() {
    let mut model = new_empty_model();
    // Test that error values are properly handled and don't break sorting
    model._set("B1", "1");
    model._set("B2", "=SQRT(-1)");  // This produces #NUM! error
    model._set("B3", "3");
    model._set("B4", "5");
    model._set("A1", "=MEDIAN(B1:B4)");
    model.evaluate();

    // Should propagate the error from B2
    assert_eq!(model._get_text("A1"), *"#NUM!");
}

#[test]
fn test_fn_median_mixed_values() {
    let mut model = new_empty_model();
    // Test median calculation with mixed numeric and text values
    model._set("B1", "1");
    model._set("B2", "\"text\"");  // String, should be ignored
    model._set("B3", "3");
    model._set("B4", "5");
    model._set("B5", "");          // Empty cell
    model._set("A1", "=MEDIAN(B1:B5)");
    model.evaluate();

    // Should return median of [1, 3, 5] = 3, ignoring text and empty cells
    assert_eq!(model._get_text("A1"), *"3");
}
