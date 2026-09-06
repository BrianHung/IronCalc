#![allow(clippy::unwrap_used)]

use crate::test::util::new_empty_model;

#[test]
fn test_fn_count_arguments() {
    let mut model = new_empty_model();
    model._set("A1", "=COUNT()");
    model._set("A2", "=COUNTA()");
    model._set("A3", "=COUNTBLANK()");
    model._set("A4", "=COUNTBLANK(C1:D1, H3:H4)");
    model.evaluate();

    assert_eq!(model._get_text("A1"), *"#ERROR!");
    assert_eq!(model._get_text("A2"), *"#ERROR!");
    assert_eq!(model._get_text("A3"), *"#ERROR!");
    assert_eq!(model._get_text("A4"), *"#ERROR!");
}

#[test]
fn test_fn_countif_date_string_criterion() {
    // 45131..=45137 -> 7/24/2023..7/30/2023 (seven dates strictly before 7/31)
    // 45138         -> 7/31/2023
    // 45139, 45200  -> after 7/31/2023
    let mut model = new_empty_model();
    for (idx, serial) in (45131..=45137).enumerate() {
        let cell = format!("B{}", idx + 2);
        model._set(&cell, &serial.to_string());
    }
    model._set("B9", "45138");
    model._set("B10", "45139");
    model._set("B11", "45200");

    model._set("A1", "=COUNTIF(B2:B11, \"<7/31/2023\")");
    model._set("A2", "=COUNTIF(B2:B11, \"<=7/31/2023\")");
    model._set("A3", "=COUNTIF(B2:B11, \"7/31/2023\")");
    model._set("A4", "=COUNTIF(B2:B11, \">7/31/2023\")");
    model._set("A5", "=COUNTIF(B2:B11, \">=7/31/2023\")");
    model._set("A6", "=COUNTIF(B2:B11, \"<>7/31/2023\")");

    model.evaluate();

    assert_eq!(model._get_text("A1"), *"7");
    assert_eq!(model._get_text("A2"), *"8");
    assert_eq!(model._get_text("A3"), *"1");
    assert_eq!(model._get_text("A4"), *"2");
    assert_eq!(model._get_text("A5"), *"3");
    assert_eq!(model._get_text("A6"), *"9");
}

/// COUNTBLANK clips its walk to the used range like every other aggregate, but
/// unlike them its answer *is* the cells the clip removes: each one lies past
/// the used range and so is blank. The clipped walk must therefore add the
/// removed area back, and add it back once -- rows, columns and both axes at
/// the same time.
///
/// Mutant: "clip without remainder" -- drop the `clipped_away_cells` term and
/// every case here collapses to the handful of blanks inside the 2x2 used
/// range. A second mutant it kills is doing the arithmetic in `i32`: a whole
/// sheet is 1,048,576 x 16,384 = 17,179,869,184 cells, which overflows.
///
/// The counts are read from a second sheet so that no formula sits inside the
/// range it measures, which would make the reference circular and move the
/// dimension it is asserting about.
#[test]
fn countblank_adds_back_the_cells_the_clip_removed() {
    let mut model = new_empty_model();
    model.add_sheet("Counts").unwrap();
    // Sheet1's used range is exactly A1:B2, all four cells non-blank.
    model._set("A1", "1");
    model._set("B1", "2");
    model._set("A2", "3");
    model._set("B2", "4");
    // Rows open, columns bounded: three full columns less the four values.
    model._set("Counts!A1", "=COUNTBLANK(Sheet1!A:C)");
    // Columns open, rows bounded: two full rows less the four values.
    model._set("Counts!A2", "=COUNTBLANK(Sheet1!1:2)");
    // Both axes open: the whole sheet less the four values, past i32.
    model._set("Counts!A3", "=COUNTBLANK(Sheet1!A:XFD)");
    model.evaluate();

    assert_eq!(
        model._get_text("Counts!A1"),
        (3 * 1_048_576 - 4).to_string()
    );
    assert_eq!(model._get_text("Counts!A2"), (2 * 16_384 - 4).to_string());
    assert_eq!(
        model._get_text("Counts!A3"),
        (1_048_576_i64 * 16_384 - 4).to_string()
    );
}

#[test]
fn test_fn_count_minimal() {
    let mut model = new_empty_model();
    model._set("B1", "3.1415926");
    model._set("B2", "Tomorrow's the day my bride's gonna come");
    model._set("B3", "'");
    model._set("A1", "=COUNT(B1:B5)");
    model._set("A2", "=COUNTA(B1:B5)");
    model._set("A3", "=COUNTBLANK(B1:B5)");
    model.evaluate();

    // There is only one number
    assert_eq!(model._get_text("A1"), *"1");
    // There are three non-empty cells
    assert_eq!(model._get_text("A2"), *"3");
    // There are 3 blank cells B4, B5 and B3 that contains the empty string
    assert_eq!(model._get_text("A3"), *"3");
}
