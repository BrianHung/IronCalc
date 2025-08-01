#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_gauss() {
    let mut model = new_empty_model();
    model._set("A1", "=GAUSS(2)");
    model._set("A2", "=GAUSS(0)");
    model._set("A3", "=GAUSS(-1)");
    model.evaluate();
    let a1: f64 = model._get_text("A1").parse().unwrap();
    let a2: f64 = model._get_text("A2").parse().unwrap();
    let a3: f64 = model._get_text("A3").parse().unwrap();
    assert!((a1 - 0.477249868).abs() < 1e-9);
    assert!(a2.abs() < 1e-12);
    assert!((a3 - (-0.341344746)).abs() < 1e-9);
}

#[test]
fn test_phi() {
    let mut model = new_empty_model();
    model._set("A1", "=PHI(0)");
    model._set("A2", "=PHI(1)");
    model._set("A3", "=PHI(-1)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"0.39894228");
    assert_eq!(model._get_text("A2"), *"0.241970725");
    assert_eq!(model._get_text("A3"), *"0.241970725"); // PHI is symmetric around 0
}

#[test]
fn test_standardize() {
    let mut model = new_empty_model();
    model._set("A1", "=STANDARDIZE(75,70,5)");
    model._set("A2", "=STANDARDIZE(65,70,5)");
    model._set("A3", "=STANDARDIZE(70,70,5)");
    model._set("A4", "=STANDARDIZE(80,70,10)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"1");
    assert_eq!(model._get_text("A2"), *"-1");
    assert_eq!(model._get_text("A3"), *"0");
    assert_eq!(model._get_text("A4"), *"1");
}

#[test]
fn test_gauss_phi_standardize_errors() {
    let mut model = new_empty_model();

    // Test wrong argument counts
    model._set("B1", "=GAUSS()"); // Too few arguments
    model._set("B2", "=GAUSS(1,2)"); // Too many arguments
    model._set("B3", "=PHI()"); // Too few arguments
    model._set("B4", "=PHI(1,2)"); // Too many arguments
    model._set("B5", "=STANDARDIZE(1,2)"); // Too few arguments
    model._set("B6", "=STANDARDIZE(1,2,3,4)"); // Too many arguments

    // Test invalid standard deviation
    model._set("B7", "=STANDARDIZE(1,2,0)"); // Zero std dev
    model._set("B8", "=STANDARDIZE(1,2,-1)"); // Negative std dev

    model.evaluate();

    // All should return errors
    assert_eq!(model._get_text("B1"), *"#ERROR!");
    assert_eq!(model._get_text("B2"), *"#ERROR!");
    assert_eq!(model._get_text("B3"), *"#ERROR!");
    assert_eq!(model._get_text("B4"), *"#ERROR!");
    assert_eq!(model._get_text("B5"), *"#ERROR!");
    assert_eq!(model._get_text("B6"), *"#ERROR!");
    assert_eq!(model._get_text("B7"), *"#NUM!"); // Should be NUM error for invalid std dev
    assert_eq!(model._get_text("B8"), *"#NUM!"); // Should be NUM error for invalid std dev
}

#[test]
fn test_extreme_values() {
    let mut model = new_empty_model();

    // Test with very large values that shouldn't cause NaN/infinity
    model._set("C1", "=GAUSS(100)"); // Very large positive value
    model._set("C2", "=GAUSS(-100)"); // Very large negative value
    model._set("C3", "=PHI(10)"); // Should be very small but valid
    model._set("C4", "=PHI(-10)"); // Should be very small but valid

    model.evaluate();

    // These should all produce valid numbers, not errors
    assert_ne!(model._get_text("C1"), *"#NUM!");
    assert_ne!(model._get_text("C2"), *"#NUM!");
    assert_ne!(model._get_text("C3"), *"#NUM!");
    assert_ne!(model._get_text("C4"), *"#NUM!");

    // Verify GAUSS approaches limits correctly
    let c1: f64 = model._get_text("C1").parse().unwrap();
    let c2: f64 = model._get_text("C2").parse().unwrap();
    assert!(c1 > 0.49); // Should approach 0.5 for large positive values
    assert!(c2 < -0.49); // Should approach -0.5 for large negative values
}

#[test]
fn test_type_errors() {
    let mut model = new_empty_model();

    // Test with text inputs
    model._set("D1", "=GAUSS(\"text\")");
    model._set("D2", "=PHI(\"text\")");
    model._set("D3", "=STANDARDIZE(\"text\",70,5)");
    model._set("D4", "=STANDARDIZE(75,\"text\",5)");
    model._set("D5", "=STANDARDIZE(75,70,\"text\")");

    model.evaluate();

    // All should return VALUE errors
    assert_eq!(model._get_text("D1"), *"#VALUE!");
    assert_eq!(model._get_text("D2"), *"#VALUE!");
    assert_eq!(model._get_text("D3"), *"#VALUE!");
    assert_eq!(model._get_text("D4"), *"#VALUE!");
    assert_eq!(model._get_text("D5"), *"#VALUE!");
}
