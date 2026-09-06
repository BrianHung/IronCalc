//! Cost witness for the whole-column / whole-row clip.
//!
//! This asserts on wall clock, not on values. That is the point: no value
//! assertion can see the change, because walking the 1,048,576 blank rows of a
//! whole-column reference gives exactly the same answer as walking the forty
//! that hold data.

use std::time::Instant;

use ironcalc_base::Model;

/// Milliseconds for 60 edit-and-recalculate cycles on a 40x4 used range with
/// `aggregates` sitting on top of it.
fn cost(aggregates: &str) -> Result<u128, String> {
    let mut model = Model::new_empty("cost", "en", "UTC", "en")?;
    // A 40x4 used range: four orders of magnitude short of the sheet.
    for row in 1..=40 {
        for column in 1..=4 {
            model.set_user_input(0, row, column, format!("{}", row * column))?;
        }
    }
    model.set_user_input(0, 1, 8, aggregates.to_string())?;
    model.evaluate();

    let start = Instant::now();
    for i in 0..60 {
        model.set_user_input(0, 1, 1, format!("{}", i % 7 + 1))?;
        model.evaluate();
    }
    Ok(start.elapsed().as_millis())
}

/// A whole-column reference spans 1,048,576 rows and a whole-row reference
/// 16,384 columns, but everything past the sheet's used range is blank. An
/// aggregate that ignores blanks must therefore cost what the same aggregate
/// over the used range costs.
///
/// The mutant is "walk the declared extent": drop the `clip_range_to_used`
/// call from any of these six walks and this workbook goes from milliseconds
/// an evaluate to hundreds of them. The bound is a 20x margin over the
/// bounded-range form measured on the same machine in the same run, plus a
/// floor, so it is well clear of the noise and still two orders of magnitude
/// under the unclipped cost.
#[test]
fn whole_column_aggregate_cost_tracks_the_used_range() -> Result<(), String> {
    // Six walks that ran to LAST_ROW/LAST_COLUMN before the clip, against the
    // same six over the used range. COUNTBLANK is in both because its clip is
    // the one that has to add a remainder back.
    let bounded = cost(
        "=COUNTA(A1:A40)+COUNTBLANK(B1:B40)+AVERAGE(A1:A40)\
         +SUBTOTAL(103,A1:A40)+MAX(A1:A40)+COUNT(A1:D5)",
    )?
    .max(1);
    let open = cost(
        "=COUNTA(A:A)+COUNTBLANK(B:B)+AVERAGE(A:A)\
         +SUBTOTAL(103,A:A)+MAX(A:A)+COUNT(1:5)",
    )?;
    println!("60 passes: bounded ranges {bounded}ms, whole column/row {open}ms");
    assert!(
        open < bounded * 20 + 400,
        "a whole-column aggregate costs more than its used range: \
         bounded={bounded}ms open={open}ms"
    );
    Ok(())
}
