#![allow(clippy::unwrap_used)]
use crate::test::util::new_empty_model;

#[test]
fn test_beta_dist_inv() {
    let mut model = new_empty_model();
    model._set("A1", "=BETA.DIST(2,8,10,TRUE,1,3)");
    model._set("A2", "=BETA.DIST(2,8,10,FALSE,1,3)");
    model._set("A3", "=BETA.INV(0.685470581,8,10,1,3)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"0.685470581");
    assert_eq!(model._get_text("A2"), *"1.483764648");
    assert_eq!(model._get_text("A3"), *"2");
}

#[test]
fn test_gamma_functions() {
    let mut model = new_empty_model();
    model._set("A1", "=GAMMA.DIST(10.00001131,9,2,FALSE)");
    model._set("A2", "=GAMMA.DIST(10.00001131,9,2,TRUE)");
    model._set("A3", "=GAMMA.INV(0.068094004,9,2)");
    model._set("A4", "=GAMMA(2.5)");
    model._set("A5", "=GAMMALN.PRECISE(4.5)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"0.03263913");
    assert_eq!(model._get_text("A2"), *"0.068094004");
    assert_eq!(model._get_text("A3"), *"10.000011314");
    assert_eq!(model._get_text("A4"), *"1.329340388");
    assert_eq!(model._get_text("A5"), *"2.453736571");
}

#[test]
fn test_expon_weibull_poisson() {
    let mut model = new_empty_model();
    model._set("A1", "=EXPON.DIST(0.2,10,TRUE)");
    model._set("A2", "=EXPON.DIST(0.2,10,FALSE)");
    model._set("A3", "=WEIBULL.DIST(105,20,100,TRUE)");
    model._set("A4", "=WEIBULL.DIST(105,20,100,FALSE)");
    model._set("A5", "=POISSON.DIST(2,5,TRUE)");
    model._set("A6", "=POISSON.DIST(2,5,FALSE)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"0.864664717");
    assert_eq!(model._get_text("A2"), *"1.353352832");
    assert_eq!(model._get_text("A3"), *"0.92958139");
    assert_eq!(model._get_text("A4"), *"0.035588864");
    assert_eq!(model._get_text("A5"), *"0.124652019");
    assert_eq!(model._get_text("A6"), *"0.084224337");
}

#[test]
fn test_binomial_functions() {
    let mut model = new_empty_model();
    model._set("A1", "=BINOM.DIST(6,10,0.5,FALSE)");
    model._set("A2", "=BINOM.DIST(6,10,0.5,TRUE)");
    model._set("A3", "=BINOM.INV(6,0.5,0.75)");
    model._set("A4", "=BINOM.DIST.RANGE(60,0.75,48)");
    model._set("A5", "=BINOM.DIST.RANGE(60,0.75,45,50)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"0.205078125");
    assert_eq!(model._get_text("A2"), *"0.828125");
    assert_eq!(model._get_text("A3"), *"4");
    assert_eq!(model._get_text("A4"), *"0.083974967");
    assert_eq!(model._get_text("A5"), *"0.523629793");
}

#[test]
fn test_statistical_functions_edge_cases() {
    let mut model = new_empty_model();

    // Test cases that would previously cause panics due to unsafe f64 to u64 conversions

    // Test negative values (should return NUM error, not panic)
    model._set("A1", "=POISSON.DIST(-1,5,TRUE)");
    model._set("A2", "=BINOM.DIST(-1,10,0.5,FALSE)");
    model._set("A3", "=BINOM.DIST.RANGE(10,0.5,-1)");

    // Test very large values exceeding u64::MAX
    model._set("B1", "=POISSON.DIST(1E20,5,TRUE)");
    model._set("B2", "=BINOM.DIST(1E20,10,0.5,FALSE)");
    model._set("B3", "=BINOM.DIST.RANGE(1E20,0.5,1)");

    // Test large trial values that exceed u64::MAX
    model._set("E1", "=BINOM.DIST(5,1E20,0.5,FALSE)");
    model._set("E2", "=BINOM.INV(1E20,0.5,0.5)");
    model._set("E3", "=BINOM.DIST.RANGE(1E20,0.5,1,2)");

    model.evaluate();

    // All of these should return #NUM! error instead of panicking
    assert!(model._get_text("A1").contains("#NUM!"));
    assert!(model._get_text("A2").contains("#NUM!"));
    assert!(model._get_text("A3").contains("#NUM!"));

    assert!(model._get_text("B1").contains("#NUM!"));
    assert!(model._get_text("B2").contains("#NUM!"));
    assert!(model._get_text("B3").contains("#NUM!"));

    assert!(model._get_text("E1").contains("#NUM!"));
    assert!(model._get_text("E2").contains("#NUM!"));
    assert!(model._get_text("E3").contains("#NUM!"));
}
