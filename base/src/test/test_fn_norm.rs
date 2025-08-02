#![allow(clippy::unwrap_used)]
use crate::test::util::new_empty_model;

#[test]
fn test_norm_functions() {
    let mut model = new_empty_model();
    model._set("A1", "=NORM.DIST(5,3,2,TRUE)");
    model._set("A2", "=NORM.DIST(5,3,2,FALSE)");
    model._set("A3", "=NORM.INV(0.5,3,2)");
    model._set("A4", "=NORM.S.DIST(1,TRUE)");
    model._set("A5", "=NORM.S.DIST(1,FALSE)");
    model._set("A6", "=NORM.S.INV(0.841344746)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"0.841344746");
    assert_eq!(model._get_text("A2"), *"0.120985362");
    assert_eq!(model._get_text("A3"), *"3");
    assert_eq!(model._get_text("A4"), *"0.841344746");
    assert_eq!(model._get_text("A5"), *"0.241970725");
    assert_eq!(model._get_text("A6"), *"1");
}

#[test]
fn test_norm_errors() {
    let mut model = new_empty_model();
    // Test wrong number of arguments
    model._set("B1", "=NORM.DIST(1,2,3)");
    // Test invalid probability range
    model._set("B2", "=NORM.S.INV(1.5)");
    // Test negative standard deviation
    model._set("B3", "=NORM.DIST(1,2,-1,TRUE)");
    // Test zero standard deviation
    model._set("B4", "=NORM.INV(0.5,3,0)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), *"#ERROR!");
    assert_eq!(model._get_text("B2"), *"#NUM!");
    assert_eq!(model._get_text("B3"), *"#NUM!");
    assert_eq!(model._get_text("B4"), *"#NUM!");
}

#[test]
fn test_norm_edge_cases() {
    let mut model = new_empty_model();
    // Test boundary probabilities (should fail)
    model._set("C1", "=NORM.INV(0,3,2)");
    model._set("C2", "=NORM.INV(1,3,2)");
    model._set("C3", "=NORM.S.INV(0)");
    model._set("C4", "=NORM.S.INV(1)");
    model.evaluate();
    assert_eq!(model._get_text("C1"), *"#NUM!");
    assert_eq!(model._get_text("C2"), *"#NUM!");
    assert_eq!(model._get_text("C3"), *"#NUM!");
    assert_eq!(model._get_text("C4"), *"#NUM!");
}

#[test]
fn test_norm_boolean_handling() {
    let mut model = new_empty_model();
    // Test various boolean representations for cumulative parameter
    model._set("D1", "=NORM.DIST(1,0,1,\"TRUE\")"); // String "TRUE"
    model._set("D2", "=NORM.DIST(1,0,1,\"false\")"); // String "false" (case insensitive)
    model._set("D3", "=NORM.DIST(1,0,1,1)"); // Number 1 (truthy)
    model._set("D4", "=NORM.DIST(1,0,1,0)"); // Number 0 (falsy)
    model._set("D5", "=NORM.DIST(1,0,1,5)"); // Number 5 (truthy)
    model._set("D6", "=NORM.S.DIST(1,\"TRUE\")"); // String "TRUE" for NORM.S.DIST
    model._set("D7", "=NORM.S.DIST(1,0)"); // Number 0 (falsy) for NORM.S.DIST
    model.evaluate();

    // D1, D3, D5 should give CDF results (TRUE cases)
    assert!(model._get_text("D1").starts_with("0.841344"));
    assert!(model._get_text("D3").starts_with("0.841344"));
    assert!(model._get_text("D5").starts_with("0.841344"));
    assert!(model._get_text("D6").starts_with("0.841344"));

    // D2, D4, D7 should give PDF results (FALSE cases)
    assert!(model._get_text("D2").starts_with("0.241970"));
    assert!(model._get_text("D4").starts_with("0.241970"));
    assert!(model._get_text("D7").starts_with("0.241970"));
}

#[test]
fn test_norm_precision() {
    let mut model = new_empty_model();
    // Test standard normal distribution special cases
    model._set("E1", "=NORM.S.DIST(0,TRUE)"); // Should be exactly 0.5
    model._set("E2", "=NORM.S.INV(0.5)"); // Should be exactly 0
    model._set("E3", "=NORM.DIST(3,3,2,TRUE)"); // Mean should be 0.5
    model._set("E4", "=NORM.INV(0.5,3,2)"); // Should be exactly mean (3)
    model.evaluate();
    assert_eq!(model._get_text("E1"), *"0.5");
    assert_eq!(model._get_text("E2"), *"0");
    assert_eq!(model._get_text("E3"), *"0.5");
    assert_eq!(model._get_text("E4"), *"3");
}

#[test]
fn test_norm_boundary_probabilities() {
    let mut model = new_empty_model();
    // Test strict boundary exclusion (0.0 and 1.0 should now fail)
    model._set("F1", "=NORM.INV(0.0,0,1)"); // Exactly 0.0 should error
    model._set("F2", "=NORM.INV(1.0,0,1)"); // Exactly 1.0 should error
    model._set("F3", "=NORM.S.INV(0.0)"); // Exactly 0.0 should error
    model._set("F4", "=NORM.S.INV(1.0)"); // Exactly 1.0 should error
    model._set("F5", "=NORM.INV(0.0000001,0,1)"); // Very small but > 0 should work
    model._set("F6", "=NORM.INV(0.9999999,0,1)"); // Very close to 1 but < 1 should work
    model.evaluate();
    assert_eq!(model._get_text("F1"), *"#NUM!");
    assert_eq!(model._get_text("F2"), *"#NUM!");
    assert_eq!(model._get_text("F3"), *"#NUM!");
    assert_eq!(model._get_text("F4"), *"#NUM!");
    // F5 and F6 should work (return numbers, not errors)
    assert!(!model._get_text("F5").contains("#"));
    assert!(!model._get_text("F6").contains("#"));
}

#[test]
fn test_norm_floating_point_validation() {
    let mut model = new_empty_model();
    // Test with very small standard deviations (near underflow)
    model._set("G1", "=NORM.DIST(1,0,0.000000001,TRUE)"); // Very small std
    model._set("G2", "=NORM.DIST(1,0,1e-100,FALSE)"); // Extremely small std

    // Test with very large values
    model._set("G3", "=NORM.DIST(1e100,0,1,TRUE)"); // Very large x
    model._set("G4", "=NORM.DIST(0,1e100,1,TRUE)"); // Very large mean

    model.evaluate();

    // Small but positive standard deviations should work
    assert!(!model._get_text("G1").contains("#"));
    assert!(!model._get_text("G2").contains("#"));

    // Large values should work (statrs should handle them)
    assert!(!model._get_text("G3").contains("#"));
    assert!(!model._get_text("G4").contains("#"));
}
