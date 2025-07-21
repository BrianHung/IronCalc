#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_fn_percentile_inc_basic() {
    let mut model = new_empty_model();
    for i in 0..5 {
        model._set(&format!("B{}", i + 1), &(i + 1).to_string());
    }
    model._set("A1", "=PERCENTILE.INC(B1:B5,0.4)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"2.6");
}

#[test]
fn test_fn_percentile_inc_boundary_k_values() {
    let mut model = new_empty_model();
    for i in 0..5 {
        model._set(&format!("B{}", i + 1), &(i + 1).to_string());
    }
    
    model._set("A1", "=PERCENTILE.INC(B1:B5,0)");
    model._set("A2", "=PERCENTILE.INC(B1:B5,1)");
    model.evaluate();
    
    assert_eq!(model._get_text("A1"), *"1"); // Min value
    assert_eq!(model._get_text("A2"), *"5"); // Max value
}

#[test]
fn test_fn_percentile_inc_invalid_k_values() {
    let mut model = new_empty_model();
    for i in 0..5 {
        model._set(&format!("B{}", i + 1), &(i + 1).to_string());
    }
    
    model._set("A1", "=PERCENTILE.INC(B1:B5,-0.1)");
    model._set("A2", "=PERCENTILE.INC(B1:B5,1.1)");
    model._set("A3", "=PERCENTILE.INC(B1:B5,1000.0)");
    model.evaluate();
    
    assert!(model._get_text("A1").contains("#NUM!"));
    assert!(model._get_text("A2").contains("#NUM!"));
    assert!(model._get_text("A3").contains("#NUM!"));
}

#[test]
fn test_fn_percentile_inc_empty_array() {
    let mut model = new_empty_model();
    model._set("A1", "=PERCENTILE.INC(B1:B1,0.5)");
    model.evaluate();
    
    assert!(model._get_text("A1").contains("#NUM!"));
}

#[test]
fn test_fn_percentile_inc_single_element() {
    let mut model = new_empty_model();
    model._set("B1", "42");
    model._set("A1", "=PERCENTILE.INC(B1:B1,0.5)");
    model.evaluate();
    
    assert_eq!(model._get_text("A1"), *"42");
}

#[test]
fn test_fn_percentile_inc_with_duplicates() {
    let mut model = new_empty_model();
    // Array with duplicates: [1, 2, 2, 3, 3, 3]
    model._set("B1", "1");
    model._set("B2", "2");
    model._set("B3", "2");
    model._set("B4", "3");
    model._set("B5", "3");
    model._set("B6", "3");
    
    model._set("A1", "=PERCENTILE.INC(B1:B6,0.5)");
    model.evaluate();
    
    assert_eq!(model._get_text("A1"), *"2.5");
}

#[test]
fn test_fn_percentile_inc_with_negative_values() {
    let mut model = new_empty_model();
    // Array with negative values: [-5, -2, 0, 2, 5]
    model._set("B1", "-5");
    model._set("B2", "-2");
    model._set("B3", "0");
    model._set("B4", "2");
    model._set("B5", "5");
    
    model._set("A1", "=PERCENTILE.INC(B1:B5,0.5)");
    model.evaluate();
    
    assert_eq!(model._get_text("A1"), *"0");
}

#[test]
fn test_fn_percentile_inc_interpolation() {
    let mut model = new_empty_model();
    // Test with array [10, 20, 30, 40]
    model._set("B1", "10");
    model._set("B2", "20");
    model._set("B3", "30");
    model._set("B4", "40");
    
    model._set("A1", "=PERCENTILE.INC(B1:B4,0.25)");
    model._set("A2", "=PERCENTILE.INC(B1:B4,0.75)");
    model.evaluate();
    
    assert_eq!(model._get_text("A1"), *"17.5");
    assert_eq!(model._get_text("A2"), *"32.5");
}

#[test]
fn test_fn_percentile_inc_wrong_argument_count() {
    let mut model = new_empty_model();
    
    model._set("A1", "=PERCENTILE.INC(B1:B5)"); // Missing k
    model._set("A2", "=PERCENTILE.INC(B1:B5,0.5,0.1)"); // Too many args
    model._set("A3", "=PERCENTILE.INC()"); // No args
    model.evaluate();
    
    assert!(model._get_text("A1").contains("#ERROR!"));
    assert!(model._get_text("A2").contains("#ERROR!"));
    assert!(model._get_text("A3").contains("#ERROR!"));
}

// ============================================================================
// PERCENTILE.EXC BASIC FUNCTIONALITY TESTS
// ============================================================================

#[test]
fn test_fn_percentile_exc_basic() {
    let mut model = new_empty_model();
    for i in 0..5 {
        model._set(&format!("B{}", i + 1), &(i + 1).to_string());
    }
    model._set("A1", "=PERCENTILE.EXC(B1:B5,0.4)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"2.4");
}

#[test]
fn test_fn_percentile_exc_boundary_k_values() {
    let mut model = new_empty_model();
    for i in 0..5 {
        model._set(&format!("B{}", i + 1), &(i + 1).to_string());
    }
    
    // Test k=0 and k=1 for PERCENTILE.EXC (should be errors)
    model._set("A1", "=PERCENTILE.EXC(B1:B5,0)");
    model._set("A2", "=PERCENTILE.EXC(B1:B5,1)");
    model.evaluate();
    
    assert!(model._get_text("A1").contains("#NUM!"));
    assert!(model._get_text("A2").contains("#NUM!"));
}

#[test]
fn test_fn_percentile_exc_invalid_k_values() {
    let mut model = new_empty_model();
    for i in 0..5 {
        model._set(&format!("B{}", i + 1), &(i + 1).to_string());
    }
    
    model._set("A1", "=PERCENTILE.EXC(B1:B5,-0.1)");
    model._set("A2", "=PERCENTILE.EXC(B1:B5,1.1)");
    model._set("A3", "=PERCENTILE.EXC(B1:B5,1000.0)");
    model.evaluate();
    
    assert!(model._get_text("A1").contains("#NUM!"));
    assert!(model._get_text("A2").contains("#NUM!"));
    assert!(model._get_text("A3").contains("#NUM!"));
}

#[test]
fn test_fn_percentile_exc_empty_array() {
    let mut model = new_empty_model();
    model._set("A1", "=PERCENTILE.EXC(B1:B1,0.5)");
    model.evaluate();
    
    assert!(model._get_text("A1").contains("#NUM!"));
}

#[test]
fn test_fn_percentile_exc_single_element() {
    let mut model = new_empty_model();
    model._set("B1", "42");
    model._set("A1", "=PERCENTILE.EXC(B1:B1,0.5)");
    model.evaluate();
    
    assert_eq!(model._get_text("A1"), *"42");
}

#[test]
fn test_fn_percentile_exc_interpolation() {
    let mut model = new_empty_model();
    // Test with array [10, 20, 30, 40]
    model._set("B1", "10");
    model._set("B2", "20");
    model._set("B3", "30");
    model._set("B4", "40");
    
    model._set("A1", "=PERCENTILE.EXC(B1:B4,0.25)");
    model._set("A2", "=PERCENTILE.EXC(B1:B4,0.75)");
    model.evaluate();
    
    assert_eq!(model._get_text("A1"), *"12.5");
    assert_eq!(model._get_text("A2"), *"37.5");
}

#[test]
fn test_fn_percentile_exc_with_duplicates() {
    let mut model = new_empty_model();
    // Array with duplicates: [1, 2, 2, 3, 3, 3]
    model._set("B1", "1");
    model._set("B2", "2");
    model._set("B3", "2");
    model._set("B4", "3");
    model._set("B5", "3");
    model._set("B6", "3");
    
    model._set("A1", "=PERCENTILE.EXC(B1:B6,0.5)");
    model.evaluate();
    
    assert_eq!(model._get_text("A1"), *"2.5");
}

#[test]
fn test_fn_percentile_exc_with_negative_values() {
    let mut model = new_empty_model();
    // Array with negative values: [-5, -2, 0, 2, 5]
    model._set("B1", "-5");
    model._set("B2", "-2");
    model._set("B3", "0");
    model._set("B4", "2");
    model._set("B5", "5");
    
    model._set("A1", "=PERCENTILE.EXC(B1:B5,0.5)");
    model.evaluate();
    
    assert_eq!(model._get_text("A1"), *"0");
}

// ============================================================================
// MIXED DATA TYPE HANDLING TESTS
// ============================================================================

#[test]
fn test_fn_percentile_with_text_data() {
    let mut model = new_empty_model();
    model._set("B1", "1");
    model._set("B2", "text");
    model._set("B3", "3");
    model._set("B4", "4");
    model._set("B5", "5");
    
    model._set("A1", "=PERCENTILE.INC(B1:B5,0.5)");
    model._set("A2", "=PERCENTILE.EXC(B1:B5,0.5)");
    model.evaluate();
    
    // Should ignore text and work with numeric values only
    assert_eq!(model._get_text("A1"), *"3.5"); // Median of [1,3,4,5]
    assert_eq!(model._get_text("A2"), *"3.5");
}

#[test]
fn test_fn_percentile_with_boolean_data() {
    let mut model = new_empty_model();
    model._set("B1", "1");
    model._set("B2", "TRUE");
    model._set("B3", "3");
    model._set("B4", "FALSE"); 
    model._set("B5", "5");
    
    model._set("A1", "=PERCENTILE.INC(B1:B5,0.5)");
    model.evaluate();
    
    // Should ignore boolean values in ranges
    assert_eq!(model._get_text("A1"), *"3"); // Median of [1,3,5]
}

// ============================================================================
// ERROR HANDLING AND EDGE CASE TESTS
// ============================================================================

#[test]
fn test_fn_percentile_invalid_range() {
    let mut model = new_empty_model();
    
    model._set("A1", "=PERCENTILE.INC(ZZ999:ZZ1000,0.5)");
    model._set("A2", "=PERCENTILE.EXC(ZZ999:ZZ1000,0.5)");
    model.evaluate();
    
    assert!(model._get_text("A1").contains("#"));
    assert!(model._get_text("A2").contains("#"));
}

#[test]
fn test_fn_percentile_very_large_k_values() {
    let mut model = new_empty_model();
    for i in 0..3 {
        model._set(&format!("B{}", i + 1), &(i + 1).to_string());
    }
    
    model._set("A1", "=PERCENTILE.INC(B1:B3,999999)");
    model._set("A2", "=PERCENTILE.EXC(B1:B3,-999999)");
    model.evaluate();
    
    assert!(model._get_text("A1").contains("#NUM!"));
    assert!(model._get_text("A2").contains("#NUM!"));
}

// ============================================================================
// PERFORMANCE AND LARGE DATASET TESTS
// ============================================================================

#[test]
fn test_fn_percentile_large_dataset_correctness() {
    let mut model = new_empty_model();
    
    // Create a larger dataset (100 values)
    for i in 1..=100 {
        model._set(&format!("B{}", i), &i.to_string());
    }
    
    model._set("A1", "=PERCENTILE.INC(B1:B100,0.95)");
    model._set("A2", "=PERCENTILE.EXC(B1:B100,0.95)");
    model.evaluate();
    
    assert_eq!(model._get_text("A1"), *"95.05");
    assert_eq!(model._get_text("A2"), *"95.95");
} 