#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_fn_xmatch_basic_exact_match() {
    let mut model = new_empty_model();
    model._set("A1", "10");
    model._set("A2", "20");
    model._set("A3", "30");
    model._set("B1", "=XMATCH(20, A1:A3)"); // Default mode 0
    model._set("B2", "=XMATCH(25, A1:A3)"); // Not found
    model.evaluate();
    assert_eq!(model._get_text("B1"), *"2");
    assert_eq!(model._get_text("B2"), *"#N/A");
}

#[test]
fn test_fn_xmatch_match_modes() {
    let mut model = new_empty_model();
    model._set("A1", "10");
    model._set("A2", "20");
    model._set("A3", "30");
    model._set("B1", "=XMATCH(25, A1:A3, -1)"); // Exact or next smaller
    model._set("B2", "=XMATCH(15, A1:A3, 1)"); // Exact or next larger
    model._set("C1", "apple");
    model._set("C2", "banana");
    model._set("C3", "apricot");
    model._set("B3", "=XMATCH(\"ap*\", C1:C3, 2)"); // Wildcard
    model._set("B4", "=XMATCH(\"^[a-c]\", C1:C3, 3)"); // Invalid match_mode
    model.evaluate();
    assert_eq!(model._get_text("B1"), *"2"); // Found 20 (next smaller than 25)
    assert_eq!(model._get_text("B2"), *"2"); // Found 20 (next larger than 15)
    assert_eq!(model._get_text("B3"), *"1"); // Found "apple"
    assert_eq!(model._get_text("B4"), *"#VALUE!");
}

#[test]
fn test_fn_xmatch_search_modes() {
    let mut model = new_empty_model();
    model._set("A1", "a");
    model._set("A2", "b");
    model._set("A3", "a"); // Duplicate
    model._set("B1", "=XMATCH(\"a\", A1:A3, 0, 1)"); // Search from first
    model._set("B2", "=XMATCH(\"a\", A1:A3, 0, -1)"); // Search from last
                                                      // Binary search tests
    model._set("C1", "10");
    model._set("C2", "20");
    model._set("C3", "30");
    model._set("B3", "=XMATCH(20, C1:C3, 0, 2)"); // Binary ascending
    model._set("D1", "30");
    model._set("D2", "20");
    model._set("D3", "10");
    model._set("B4", "=XMATCH(20, D1:D3, 0, -2)"); // Binary descending
    model.evaluate();
    assert_eq!(model._get_text("B1"), *"1"); // First occurrence
    assert_eq!(model._get_text("B2"), *"3"); // Last occurrence
    assert_eq!(model._get_text("B3"), *"2"); // Binary search ascending
    assert_eq!(model._get_text("B4"), *"2"); // Binary search descending
}

#[test]
fn test_fn_xmatch_data_types_and_vectors() {
    let mut model = new_empty_model();
    // Different data types
    model._set("A1", "1.5");
    model._set("A2", "hello");
    model._set("A3", "TRUE");
    model._set("B1", "=XMATCH(\"hello\", A1:A3)");
    model._set("B2", "=XMATCH(TRUE, A1:A3)");
    // Column vector
    model._set("C1", "apple");
    model._set("D1", "banana");
    model._set("E1", "cherry");
    model._set("B3", "=XMATCH(\"banana\", C1:E1)");
    // Empty cells
    model._set("F1", "");
    model._set("F2", "test");
    model._set("B4", "=XMATCH(\"\", F1:F2)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), *"2");
    assert_eq!(model._get_text("B2"), *"3");
    assert_eq!(model._get_text("B3"), *"2"); // Column vector
    assert_eq!(model._get_text("B4"), *"1"); // Empty cell
}

#[test]
fn test_fn_xmatch_error_conditions() {
    let mut model = new_empty_model();
    model._set("A1", "test");
    // Wrong number of arguments
    model._set("B1", "=XMATCH(\"test\")");
    model._set("B2", "=XMATCH(\"test\", A1:A1, 0, 1, 1)");
    // Invalid modes
    model._set("B3", "=XMATCH(\"test\", A1:A1, 5)"); // Invalid match mode
    model._set("B4", "=XMATCH(\"test\", A1:A1, 0, 5)"); // Invalid search mode
                                                        // Non-vector range
    model._set("A2", "test2");
    model._set("C1", "test3");
    model._set("C2", "test4");
    model._set("B5", "=XMATCH(\"test\", A1:C2)"); // 2x2 range
                                                  // Binary search with wildcard (should error)
    model._set("B6", "=XMATCH(\"ap*\", A1:A1, 2, 2)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), *"#ERROR!");
    assert_eq!(model._get_text("B2"), *"#ERROR!");
    assert_eq!(model._get_text("B3"), *"#VALUE!");
    assert_eq!(model._get_text("B4"), *"#ERROR!");
    assert_eq!(model._get_text("B5"), *"#ERROR!");
    assert_eq!(model._get_text("B6"), *"#VALUE!");
}

#[test]
fn test_fn_xmatch_edge_cases() {
    let mut model = new_empty_model();
    // Case sensitivity (case-insensitive comparison)
    model._set("A1", "Test");
    model._set("A2", "TEST");
    model._set("A3", "test");
    model._set("B1", "=XMATCH(\"test\", A1:A3)");
    // No smaller/larger values available
    model._set("C1", "20");
    model._set("C2", "30");
    model._set("C3", "40");
    model._set("B2", "=XMATCH(10, C1:C3, -1)"); // No smaller
    model._set("B3", "=XMATCH(50, C1:C3, 1)"); // No larger
    // Invalid match_mode
    model._set("B4", "=XMATCH(\"[\", A1:A1, 3)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), *"1"); // Case-insensitive
    assert_eq!(model._get_text("B2"), *"#N/A"); // No smaller value
    assert_eq!(model._get_text("B3"), *"#N/A"); // No larger value
    assert_eq!(model._get_text("B4"), *"#VALUE!"); // Invalid match_mode
}

#[test]
fn test_fn_xmatch_range_as_lookup_value() {
    let mut model = new_empty_model();
    model._set("A1", "10");
    model._set("A2", "20");
    model._set("B1", "10");
    model._set("B2", "20");
    model._set("B3", "30");

    // Test passing a range as lookup_value (should error since implicit intersection not supported)
    model._set("C1", "=XMATCH(A1:A2, B1:B3)"); // Range as first argument should error
    model._set("C2", "=XMATCH(A1:A1, B1:B3)"); // Single-cell range as first argument should also error

    model.evaluate();

    // Since implicit intersection isn't supported, these return #N/A
    assert_eq!(model._get_text("C1"), *"#N/A");
    assert_eq!(model._get_text("C2"), *"#N/A");
}
