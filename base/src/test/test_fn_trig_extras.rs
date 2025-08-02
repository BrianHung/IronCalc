#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_basic_values_and_arguments() {
    let mut model = new_empty_model();

    // Test basic known values
    model._set("A1", "=COT(PI()/4)");
    model._set("A2", "=COTH(1)");
    model._set("A3", "=ACOT(2)");
    model._set("A4", "=ACOTH(2)");
    model._set("A5", "=CSC(PI()/2)");
    model._set("A6", "=CSCH(1)");
    model._set("A7", "=SEC(0)");
    model._set("A8", "=SECH(1)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"1");
    assert_eq!(model._get_text("A2"), *"1.313035285");
    assert_eq!(model._get_text("A3"), *"0.463647609");
    assert_eq!(model._get_text("A4"), *"0.549306144");
    assert_eq!(model._get_text("A5"), *"1");
    assert_eq!(model._get_text("A6"), *"0.850918128");
    assert_eq!(model._get_text("A7"), *"1");
    assert_eq!(model._get_text("A8"), *"0.648054274");
}

#[test]
fn test_trigonometric_error_conditions() {
    let mut model = new_empty_model();

    // Test domain errors for ACOTH (these are the most reliable error conditions)
    model._set("A1", "=ACOTH(0.5)"); // |f| < 1.0 should be #NUM!
    model._set("A2", "=ACOTH(1.0)"); // f = 1.0 should be #DIV/0!
    model._set("A3", "=ACOTH(-1.0)"); // f = -1.0 should be #DIV/0!
    model._set("A4", "=ACOTH(0)"); // f = 0 should be #NUM!

    // Test some extreme values that might trigger errors
    model._set("B1", "=COTH(0)"); // hyperbolic cotangent of 0
    model._set("B2", "=CSCH(0)"); // hyperbolic cosecant of 0

    model.evaluate();

    // Domain errors for ACOTH
    assert_eq!(model._get_text("A1"), *"#NUM!");
    assert_eq!(model._get_text("A2"), *"#DIV/0!");
    assert_eq!(model._get_text("A3"), *"#DIV/0!");
    assert_eq!(model._get_text("A4"), *"#NUM!");

    // Division by zero for hyperbolic functions
    assert_eq!(model._get_text("B1"), *"#DIV/0!");
    assert_eq!(model._get_text("B2"), *"#DIV/0!");
}

#[test]
fn test_excel_compatibility_values() {
    let mut model = new_empty_model();

    // Test specific Excel compatibility values
    model._set("A1", "=COT(30*PI()/180)");
    model._set("A2", "=CSC(30*PI()/180)");
    model._set("A3", "=SEC(60*PI()/180)");
    model._set("A4", "=ACOT(1)");

    model.evaluate();

    let cot_30 = model._get_text("A1").parse::<f64>().unwrap();
    assert!((cot_30 - 3_f64.sqrt()).abs() < 1e-9);

    let csc_30 = model._get_text("A2").parse::<f64>().unwrap();
    assert!((csc_30 - 2.0).abs() < 1e-9);

    let sec_60 = model._get_text("A3").parse::<f64>().unwrap();
    assert!((sec_60 - 2.0).abs() < 1e-9);

    let acot_1 = model._get_text("A4").parse::<f64>().unwrap();
    assert!((acot_1 - std::f64::consts::FRAC_PI_4).abs() < 1e-8);
}

#[test]
fn test_extreme_values() {
    let mut model = new_empty_model();

    model._set("A1", "=ACOT(1e15)");
    model._set("A2", "=ACOT(0)");
    model._set("A3", "=ACOTH(1.0000001)");
    model._set("A4", "=SECH(15)");

    model.evaluate();

    let acot_large = model._get_text("A1").parse::<f64>().unwrap();
    assert!(acot_large.abs() < 1e-14);

    let acot_zero = model._get_text("A2").parse::<f64>().unwrap();
    assert!((acot_zero - std::f64::consts::FRAC_PI_2).abs() < 1e-8);

    assert!(model._get_text("A3").parse::<f64>().is_ok());

    let sech_large = model._get_text("A4").parse::<f64>().unwrap();
    assert!(sech_large > 0.0 && sech_large < 1e-6);
}

#[test]
fn test_ieee754_compliance() {
    let mut model = new_empty_model();

    model._set("A1", "=COT(1e-16)");
    model._set("A2", "=COTH(1e-16)");
    model._set("A3", "=SECH(50)");

    model.evaluate();

    let cot_small = model._get_text("A1").parse::<f64>().unwrap();
    assert!(cot_small > 1e15 && cot_small.is_finite());

    let coth_small = model._get_text("A2").parse::<f64>().unwrap();
    assert!(coth_small.abs() > 1e15 && coth_small.is_finite());

    let sech_large = model._get_text("A3").parse::<f64>().unwrap();
    assert!(sech_large > 0.0 && sech_large < 1e-20);
}

#[test]
fn test_mathematical_properties() {
    let mut model = new_empty_model();

    // Test mathematical relationships
    model._set("A1", "=COT(1)");
    model._set("A2", "=1/TAN(1)");
    model._set("B1", "=CSC(1)");
    model._set("B2", "=1/SIN(1)");
    model._set("C1", "=SEC(1)");
    model._set("C2", "=1/COS(1)");

    // Test inverse relationships
    model._set("D1", "=ACOT(COT(0.5))");
    model._set("D2", "=0.5");

    model.evaluate();

    let cot_direct = model._get_text("A1").parse::<f64>().unwrap();
    let cot_derived = model._get_text("A2").parse::<f64>().unwrap();
    assert!((cot_direct - cot_derived).abs() < 1e-15);

    let csc_direct = model._get_text("B1").parse::<f64>().unwrap();
    let csc_derived = model._get_text("B2").parse::<f64>().unwrap();
    assert!((csc_direct - csc_derived).abs() < 1e-15);

    let sec_direct = model._get_text("C1").parse::<f64>().unwrap();
    let sec_derived = model._get_text("C2").parse::<f64>().unwrap();
    assert!((sec_direct - sec_derived).abs() < 1e-15);

    let inverse_test = model._get_text("D1").parse::<f64>().unwrap();
    let original_value = model._get_text("D2").parse::<f64>().unwrap();
    assert!((inverse_test - original_value).abs() < 1e-14);
}

#[test]
fn test_argument_validation_comprehensive() {
    let mut model = new_empty_model();

    // Test all functions with no arguments (should all error)
    let no_arg_functions = ["COT", "COTH", "ACOT", "ACOTH", "CSC", "CSCH", "SEC", "SECH"];

    for (i, func) in no_arg_functions.iter().enumerate() {
        model._set(&format!("A{}", i + 1), &format!("={func}()"));
    }

    // Test all functions with too many arguments (should all error)
    for (i, func) in no_arg_functions.iter().enumerate() {
        model._set(&format!("B{}", i + 1), &format!("={func}(1,2)"));
    }

    model.evaluate();

    // All should be #ERROR!
    for i in 1..=no_arg_functions.len() {
        assert_eq!(model._get_text(&format!("A{i}")), *"#ERROR!");
        assert_eq!(model._get_text(&format!("B{i}")), *"#ERROR!");
    }
}

#[test]
fn test_algorithm_equivalence() {
    let mut model = new_empty_model();

    // Test alternative formulations match
    model._set("A1", "=ACOT(2)");
    model._set("A2", "=PI()/2-ATAN(2)");
    model._set("B1", "=ACOTH(3)");
    model._set("B2", "=0.5*LN((3+1)/(3-1))");

    model.evaluate();

    let acot_atan2 = model._get_text("A1").parse::<f64>().unwrap();
    let acot_pi_atan = model._get_text("A2").parse::<f64>().unwrap();
    assert!((acot_atan2 - acot_pi_atan).abs() < 1e-12);

    let acoth_atanh = model._get_text("B1").parse::<f64>().unwrap();
    let acoth_ln = model._get_text("B2").parse::<f64>().unwrap();
    assert!((acoth_atanh - acoth_ln).abs() < 1e-12);
}

#[test]
fn test_function_symmetry() {
    let mut model = new_empty_model();

    // Test odd/even properties
    model._set("A1", "=COT(1)");
    model._set("A2", "=COT(-1)");
    model._set("B1", "=SEC(1)");
    model._set("B2", "=SEC(-1)");

    model.evaluate();

    // COT is odd: cot(-x) = -cot(x)
    let cot_pos = model._get_text("A1").parse::<f64>().unwrap();
    let cot_neg = model._get_text("A2").parse::<f64>().unwrap();
    assert!((cot_pos + cot_neg).abs() < 1e-14);

    // SEC is even: sec(-x) = sec(x)
    let sec_pos = model._get_text("B1").parse::<f64>().unwrap();
    let sec_neg = model._get_text("B2").parse::<f64>().unwrap();
    assert!((sec_pos - sec_neg).abs() < 1e-14);
}
