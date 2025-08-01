#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn ceiling_math_examples() {
    let mut model = new_empty_model();
    model._set("A1", "=CEILING.MATH(24.3,5)");
    model._set("A2", "=CEILING.MATH(6.7)");
    model._set("A3", "=CEILING.MATH(-8.1,2)");
    model._set("A4", "=CEILING.MATH(-5.5,2,-1)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"25");
    assert_eq!(model._get_text("A2"), *"7");
    assert_eq!(model._get_text("A3"), *"-8");
    assert_eq!(model._get_text("A4"), *"-6");
}

#[test]
fn ceiling_precise_and_iso() {
    let mut model = new_empty_model();
    model._set("A1", "=CEILING.PRECISE(4.3)");
    model._set("A2", "=CEILING.PRECISE(-4.3)");
    model._set("A3", "=ISO.CEILING(4.3,2)");

    // Test with _xlfn prefixes for Excel compatibility
    model._set("A4", "=_XLFN.CEILING.PRECISE(4.3)");
    model._set("A5", "=_XLFN.ISO.CEILING(4.3,2)");

    model.evaluate();
    assert_eq!(model._get_text("A1"), *"5");
    assert_eq!(model._get_text("A2"), *"-4");
    assert_eq!(model._get_text("A3"), *"6");
    assert_eq!(model._get_text("A4"), *"5"); // Should work with _xlfn prefix
    assert_eq!(model._get_text("A5"), *"6"); // Should work with _xlfn prefix
}

#[test]
fn floor_math_and_precise() {
    let mut model = new_empty_model();
    model._set("A1", "=FLOOR.MATH(24.3,5)");
    model._set("A2", "=FLOOR.MATH(-8.1,2)");
    model._set("A3", "=FLOOR.MATH(-5.5,2,-1)");
    model._set("A4", "=FLOOR.PRECISE(-4.3)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"20");
    assert_eq!(model._get_text("A2"), *"-10");
    assert_eq!(model._get_text("A3"), *"-4");
    assert_eq!(model._get_text("A4"), *"-4");
}

#[test]
fn ceiling_floor_edge_cases() {
    let mut model = new_empty_model();

    // Test with zero significance
    model._set("A1", "=CEILING.MATH(5,0)");
    model._set("A2", "=FLOOR.MATH(5,0)");

    // Test exact multiples (should not change)
    model._set("A3", "=CEILING.MATH(10,5)");
    model._set("A4", "=FLOOR.MATH(10,5)");

    // Test very small numbers
    model._set("A5", "=CEILING.MATH(0.1,0.25)");
    model._set("A6", "=FLOOR.MATH(0.1,0.25)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"0"); // Zero significance returns 0
    assert_eq!(model._get_text("A2"), *"0");
    assert_eq!(model._get_text("A3"), *"10"); // Exact multiple
    assert_eq!(model._get_text("A4"), *"10");
    assert_eq!(model._get_text("A5"), *"0.25"); // Round up to nearest quarter
    assert_eq!(model._get_text("A6"), *"0"); // Round down to nearest quarter
}

#[test]
fn ceiling_math_comprehensive() {
    let mut model = new_empty_model();

    // From Patch 5 - more comprehensive test cases
    model._set("A1", "=CEILING.MATH(24.3,5)");
    model._set("A2", "=CEILING.MATH(6.7)");
    model._set("A3", "=CEILING.MATH(-8.1,2)");
    model._set("A4", "=CEILING.MATH(-5.5,2,-1)");

    // Additional edge cases
    model._set("A5", "=CEILING.MATH(0)");
    model._set("A6", "=CEILING.MATH(-0)");
    model._set("A7", "=CEILING.MATH(4.3,2)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"25");
    assert_eq!(model._get_text("A2"), *"7");
    assert_eq!(model._get_text("A3"), *"-8");
    assert_eq!(model._get_text("A4"), *"-6");
    assert_eq!(model._get_text("A5"), *"0");
    assert_eq!(model._get_text("A6"), *"0");
    assert_eq!(model._get_text("A7"), *"6");
}

#[test]
fn floor_comprehensive() {
    let mut model = new_empty_model();

    // From Patch 5 - comprehensive floor tests
    model._set("A1", "=FLOOR.MATH(24.3,5)");
    model._set("A2", "=FLOOR.MATH(6.7)");
    model._set("A3", "=FLOOR.MATH(-8.1,2)");
    model._set("A4", "=FLOOR.MATH(-5.5,2,-1)");
    model._set("A5", "=FLOOR.PRECISE(4.3)");
    model._set("A6", "=FLOOR.PRECISE(-4.3)");
    model._set("A7", "=FLOOR.PRECISE(4.3,2)");
    model._set("A8", "=FLOOR.PRECISE(-4.3,2)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"20");
    assert_eq!(model._get_text("A2"), *"6");
    assert_eq!(model._get_text("A3"), *"-10");
    assert_eq!(model._get_text("A4"), *"-4");
    assert_eq!(model._get_text("A5"), *"4");
    assert_eq!(model._get_text("A6"), *"-4");
    assert_eq!(model._get_text("A7"), *"4");
    assert_eq!(model._get_text("A8"), *"-4");
}

#[test]
fn default_significance_behavior() {
    let mut model = new_empty_model();

    // Test default significance: +1.0 for positive, -1.0 for negative
    model._set("A1", "=FLOOR.MATH(6.7)"); // positive -> significance 1.0
    model._set("A2", "=FLOOR.MATH(-6.7)"); // negative -> significance -1.0
    model._set("A3", "=CEILING.MATH(6.7)"); // positive -> significance 1.0
    model._set("A4", "=CEILING.MATH(-6.7)"); // negative -> significance -1.0

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"6"); // 6.7 floors to 6
    assert_eq!(model._get_text("A2"), *"-7"); // -6.7 floors away from zero to -7
    assert_eq!(model._get_text("A3"), *"7"); // 6.7 ceilings to 7
    assert_eq!(model._get_text("A4"), *"-6"); // -6.7 ceilings toward zero to -6
}

#[test]
fn excel_compatibility_xlfn_prefixes() {
    let mut model = new_empty_model();

    // Test all functions work with _xlfn prefixes
    model._set("A1", "=_XLFN.CEILING.MATH(24.3,5)");
    model._set("A2", "=_XLFN.CEILING.PRECISE(4.3)");
    model._set("A3", "=_XLFN.ISO.CEILING(4.3,2)");
    model._set("A4", "=_XLFN.FLOOR.MATH(24.3,5)");
    model._set("A5", "=_XLFN.FLOOR.PRECISE(-4.3)");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"25");
    assert_eq!(model._get_text("A2"), *"5");
    assert_eq!(model._get_text("A3"), *"6");
    assert_eq!(model._get_text("A4"), *"20");
    assert_eq!(model._get_text("A5"), *"-4");
}
