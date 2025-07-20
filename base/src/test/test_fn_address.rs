#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn basic_address() {
    let mut model = new_empty_model();
    model._set("A1", "=ADDRESS(1,1)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"$A$1");
}

#[test]
fn address_with_sheet_and_r1c1() {
    let mut model = new_empty_model();
    model.new_sheet();
    model._set("A1", "=ADDRESS(4,3,2,FALSE,\"Sheet2\")");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"Sheet2!R4C[3]");
}

#[test]
fn address_invalid() {
    let mut model = new_empty_model();
    model._set("A1", "=ADDRESS(0,1)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#VALUE!");
}

// Test all abs_num values (1-4) with A1 style
#[test]
fn address_abs_num_a1_style() {
    let mut model = new_empty_model();
    model._set("A1", "=ADDRESS(5,3,1)"); // Both absolute
    model._set("A2", "=ADDRESS(5,3,2)"); // Row absolute
    model._set("A3", "=ADDRESS(5,3,3)"); // Column absolute
    model._set("A4", "=ADDRESS(5,3,4)"); // Both relative
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"$C$5");
    assert_eq!(model._get_text("A2"), *"C$5");
    assert_eq!(model._get_text("A3"), *"$C5");
    assert_eq!(model._get_text("A4"), *"C5");
}

// Test all abs_num values (1-4) with R1C1 style
#[test]
fn address_abs_num_r1c1_style() {
    let mut model = new_empty_model();
    model._set("A1", "=ADDRESS(5,3,1,FALSE)"); // Both absolute
    model._set("A2", "=ADDRESS(5,3,2,FALSE)"); // Row absolute
    model._set("A3", "=ADDRESS(5,3,3,FALSE)"); // Column absolute
    model._set("A4", "=ADDRESS(5,3,4,FALSE)"); // Both relative
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"R5C3");
    assert_eq!(model._get_text("A2"), *"R5C[3]");
    assert_eq!(model._get_text("A3"), *"R[5]C3");
    assert_eq!(model._get_text("A4"), *"R[5]C[3]");
}

// Test with sheet names
#[test]
fn address_with_sheet_names() {
    let mut model = new_empty_model();
    model._set("A1", "=ADDRESS(10,5,1,TRUE,\"MySheet\")"); // A1 style
    model._set("A2", "=ADDRESS(10,5,1,FALSE,\"MySheet\")"); // R1C1 style
    model._set("A3", "=ADDRESS(2,1,1,TRUE,\"My Sheet\")"); // Sheet with spaces
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"MySheet!$E$10");
    assert_eq!(model._get_text("A2"), *"MySheet!R10C5");
    assert_eq!(model._get_text("A3"), *"'My Sheet'!$A$2");
}

// Test edge cases for row/column limits
#[test]
fn address_boundary_values() {
    let mut model = new_empty_model();
    model._set("A1", "=ADDRESS(1,1)"); // Min values
    model._set("A2", "=ADDRESS(1048576,16384)"); // Max values
    model._set("A3", "=ADDRESS(1,26)"); // Column Z
    model._set("A4", "=ADDRESS(1,27)"); // Column AA
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"$A$1");
    assert_eq!(model._get_text("A2"), *"$XFD$1048576");
    assert_eq!(model._get_text("A3"), *"$Z$1");
    assert_eq!(model._get_text("A4"), *"$AA$1");
}

// Test invalid inputs
#[test]
fn address_invalid_inputs() {
    let mut model = new_empty_model();
    model._set("A1", "=ADDRESS(0,1)"); // Invalid row
    model._set("A2", "=ADDRESS(1,0)"); // Invalid column
    model._set("A3", "=ADDRESS(1048577,1)"); // Row too large
    model._set("A4", "=ADDRESS(1,16385)"); // Column too large
    model._set("A5", "=ADDRESS(1,1,0)"); // Invalid abs_num
    model._set("A6", "=ADDRESS(1,1,5)"); // Invalid abs_num
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#VALUE!");
    assert_eq!(model._get_text("A2"), *"#VALUE!");
    assert_eq!(model._get_text("A3"), *"#VALUE!");
    assert_eq!(model._get_text("A4"), *"#VALUE!");
    assert_eq!(model._get_text("A5"), *"#VALUE!");
    assert_eq!(model._get_text("A6"), *"#VALUE!");
}

// Test argument count and type errors
#[test]
fn address_too_few_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=ADDRESS(1)"); // Too few arguments
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#ERROR!");
}

#[test]
fn address_too_many_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=ADDRESS(1,1,1,TRUE,\"Sheet\",\"Extra\")"); // Too many arguments
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#ERROR!");
}

#[test]
fn address_string_row() {
    let mut model = new_empty_model();
    model._set("A1", "=ADDRESS(\"text\",1)"); // String row
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#VALUE!");
}

#[test]
fn address_string_column() {
    let mut model = new_empty_model();
    model._set("A1", "=ADDRESS(1,\"text\")"); // String column
    model.evaluate();
    assert_eq!(model._get_text("A1"), *"#VALUE!");
}
