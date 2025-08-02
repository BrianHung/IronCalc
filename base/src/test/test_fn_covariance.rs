#![allow(clippy::unwrap_used)]
use crate::test::util::new_empty_model;

#[test]
fn test_covariance_functions() {
    let mut model = new_empty_model();
    model._set("B1", "3");
    model._set("B2", "2");
    model._set("B3", "4");
    model._set("B4", "5");
    model._set("B5", "6");

    model._set("C1", "9");
    model._set("C2", "7");
    model._set("C3", "12");
    model._set("C4", "15");
    model._set("C5", "17");

    model._set("A1", "=COVARIANCE.P(B1:B5,C1:C5)");
    model._set("A2", "=COVARIANCE.S(B1:B5,C1:C5)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"5.2");
    assert_eq!(model._get_text("A2"), *"6.5");
}

#[test]
fn test_covariance_errors() {
    let mut model = new_empty_model();
    // Test mismatched array sizes
    model._set("A1", "1");
    model._set("B1", "2");
    model._set("B2", "3");
    model._set("D1", "=COVARIANCE.P(A1:A1,B1:B2)");

    // Test insufficient data for sample covariance
    model._set("D2", "=COVARIANCE.S(A1:A1,B1:B1)");

    // Test empty data
    model._set("E1", "");
    model._set("E2", "");
    model._set("D3", "=COVARIANCE.P(E1:E1,E2:E2)");

    model.evaluate();
    assert_eq!(model._get_text("D1"), *"#N/A");
    assert_eq!(model._get_text("D2"), *"#DIV/0!");
    assert_eq!(model._get_text("D3"), *"#DIV/0!");
}

#[test]
fn test_covariance_with_mixed_types() {
    let mut model = new_empty_model();
    // Test with mixed types (numbers, strings, booleans)
    model._set("F1", "1");
    model._set("F2", "text");
    model._set("F3", "3");
    model._set("G1", "2");
    model._set("G2", "4");
    model._set("G3", "6");

    model._set("H1", "=COVARIANCE.P(F1:F3,G1:G3)");
    model.evaluate();
    // Should only use the numeric pairs (1,2) and (3,6)
    // This tests the robust data collection from Patch 4
    assert_eq!(model._get_text("H1"), *"2");
}

#[test]
fn test_covariance_robustness() {
    let mut model = new_empty_model();

    // Test with extremely large values
    model._set("I1", "1000000000");
    model._set("I2", "2000000000");
    model._set("J1", "3000000000");
    model._set("J2", "4000000000");
    model._set("K1", "=COVARIANCE.P(I1:I2,J1:J2)");

    // Test with very small values near zero
    model._set("L1", "0.000000001");
    model._set("L2", "0.000000002");
    model._set("M1", "0.000000003");
    model._set("M2", "0.000000004");
    model._set("K2", "=COVARIANCE.P(L1:L2,M1:M2)");

    // Test perfect correlation (should give positive covariance)
    model._set("N1", "1");
    model._set("N2", "2");
    model._set("N3", "3");
    model._set("O1", "10");
    model._set("O2", "20");
    model._set("O3", "30");
    model._set("K3", "=COVARIANCE.P(N1:N3,O1:O3)");

    // Test perfect negative correlation
    model._set("P1", "1");
    model._set("P2", "2");
    model._set("P3", "3");
    model._set("Q1", "30");
    model._set("Q2", "20");
    model._set("Q3", "10");
    model._set("K4", "=COVARIANCE.P(P1:P3,Q1:Q3)");

    model.evaluate();

    // All should return valid numbers (not errors)
    assert!(!model._get_text("K1").contains("#"));
    assert!(!model._get_text("K2").contains("#"));
    assert!(!model._get_text("K3").contains("#"));
    assert!(!model._get_text("K4").contains("#"));

    // Perfect positive correlation should be positive
    let pos_cov: f64 = model._get_text("K3").parse().unwrap_or(0.0);
    assert!(pos_cov > 0.0);

    // Perfect negative correlation should be negative
    let neg_cov: f64 = model._get_text("K4").parse().unwrap_or(0.0);
    assert!(neg_cov < 0.0);
}

#[test]
fn test_covariance_zero_variance() {
    let mut model = new_empty_model();

    // Test with constant values (zero variance)
    model._set("R1", "5");
    model._set("R2", "5");
    model._set("R3", "5");
    model._set("S1", "10");
    model._set("S2", "15");
    model._set("S3", "20");
    model._set("T1", "=COVARIANCE.P(R1:R3,S1:S3)");

    // Test with both constant (should be zero)
    model._set("U1", "5");
    model._set("U2", "5");
    model._set("V1", "10");
    model._set("V2", "10");
    model._set("T2", "=COVARIANCE.P(U1:U2,V1:V2)");

    model.evaluate();

    // Constant vs varying should give zero covariance
    assert_eq!(model._get_text("T1"), *"0");

    // Both constant should give zero covariance
    assert_eq!(model._get_text("T2"), *"0");
}
