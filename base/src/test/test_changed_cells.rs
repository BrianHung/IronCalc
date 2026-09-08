//! Witnesses for the change delta: what [`Model::take_changed_cells`] promises
//! a reader repainting a workbook.

#![allow(clippy::unwrap_used)]
// A helper that cannot produce the value the test asked for fails the test.
#![allow(clippy::panic)]
// The timing witness reports its measurements, not just its verdict.
#![allow(clippy::print_stdout)]

use std::time::Instant;

use crate::cell::CellValue;
use crate::expressions::types::CellReferenceIndex;
use crate::types::Link;
use crate::{ChangedSinceRead, Model};

fn at(row: i32, column: i32) -> CellReferenceIndex {
    CellReferenceIndex {
        sheet: 0,
        row,
        column,
    }
}

/// The delta, or a failure naming the `Everything` the test did not expect.
fn delta(model: &mut Model) -> Vec<CellReferenceIndex> {
    match model.take_changed_cells() {
        ChangedSinceRead::Cells(cells) => cells,
        ChangedSinceRead::Everything => panic!("expected a delta, got Everything"),
    }
}

/// The bit pattern of a numeric cell, so `+0.0` and `-0.0` are told apart.
fn number_bits(model: &Model, row: i32, column: i32) -> u64 {
    match model.get_cell_value_by_index(0, row, column).unwrap() {
        CellValue::Number(n) => n.to_bits(),
        other => panic!("expected a number, got {other:?}"),
    }
}

/// A model whose delta is being tracked, past its opening `Everything`.
fn tracked_model() -> Model<'static> {
    let mut model = Model::new_empty("m", "en", "UTC", "en").unwrap();
    model.evaluate();
    assert_eq!(model.take_changed_cells(), ChangedSinceRead::Everything);
    model
}

/// Completeness and soundness on one edit chain: every cell whose value the
/// edit moved is listed, and a cell it did not touch is not.
#[test]
fn an_edit_chain_lists_the_cells_that_moved_and_only_those() {
    let mut model = tracked_model();
    model.set_user_input(0, 1, 1, "1".to_string()).unwrap();
    model.set_user_input(0, 1, 2, "=A1*2".to_string()).unwrap();
    model.set_user_input(0, 1, 3, "=B1+1".to_string()).unwrap();
    model.set_user_input(0, 1, 4, "99".to_string()).unwrap();
    model.evaluate();
    let _ = delta(&mut model);

    model.set_user_input(0, 1, 1, "5".to_string()).unwrap();
    model.evaluate();

    // A1 is the edit the journal saw; B1 and C1 are values the pass moved and
    // only the snapshot diff can see. D1 was recomputed and did not move.
    assert_eq!(delta(&mut model), vec![at(1, 1), at(1, 2), at(1, 3)]);
    assert_eq!(number_bits(&model, 1, 2), 10.0f64.to_bits());
    assert_eq!(number_bits(&model, 1, 3), 11.0f64.to_bits());
    assert_eq!(number_bits(&model, 1, 4), 99.0f64.to_bits());

    // Reading clears the record: an idle pass reports an empty delta, which is
    // not the same answer as `Everything`.
    model.evaluate();
    assert_eq!(delta(&mut model), vec![]);
}

/// A `HYPERLINK` target can move while the label the cell displays does not.
/// The link is part of the cell's content, so the cell is listed.
#[test]
fn a_hyperlink_target_moving_under_an_unchanged_label_is_listed() {
    let mut model = tracked_model();
    model
        .set_user_input(0, 1, 1, "http://a.example".to_string())
        .unwrap();
    model
        .set_user_input(0, 1, 2, "=HYPERLINK(A1, \"click\")".to_string())
        .unwrap();
    model.evaluate();
    let _ = delta(&mut model);

    // Only A1 is edited. B1 is not written at all, and its value does not
    // move, so the link in its key is the only thing that can list it.
    model
        .set_user_input(0, 1, 1, "http://b.example".to_string())
        .unwrap();
    model.evaluate();

    // B1 still displays "click": nothing but the link moved.
    assert_eq!(
        model.get_cell_value_by_index(0, 1, 2),
        Ok(CellValue::String("click".to_string()))
    );
    assert!(delta(&mut model).contains(&at(1, 2)));
}

/// `+0.0 == -0.0`, so a key that compared values would report a sign flip as
/// no change at all. The key compares bits.
#[test]
fn a_signed_zero_flip_is_listed() {
    let mut model = tracked_model();
    model.set_user_input(0, 1, 1, "1".to_string()).unwrap();
    model.set_user_input(0, 1, 2, "=0*A1".to_string()).unwrap();
    model.evaluate();
    let _ = delta(&mut model);
    assert_eq!(number_bits(&model, 1, 2), 0.0f64.to_bits());

    model.set_user_input(0, 1, 1, "-1".to_string()).unwrap();
    model.evaluate();

    assert_eq!(number_bits(&model, 1, 2), (-0.0f64).to_bits());
    assert!(delta(&mut model).contains(&at(1, 2)));
}

/// The delta reports cell content, not styling. A quote prefix is stored as a
/// style flag but reads back as content, so it is the one flip that counts.
#[test]
fn a_style_only_edit_is_not_listed_but_a_quote_prefix_flip_is() {
    let mut model = tracked_model();
    model.set_user_input(0, 1, 1, "abc".to_string()).unwrap();
    model.set_user_input(0, 2, 1, "keep".to_string()).unwrap();
    model.evaluate();
    let _ = delta(&mut model);

    let mut style = model.get_style_for_cell(0, 2, 1).unwrap();
    style.font.b = true;
    model.set_cell_style(0, 2, 1, &style).unwrap();
    model.evaluate();
    assert_eq!(delta(&mut model), vec![]);

    model.set_user_input(0, 1, 1, "'abc".to_string()).unwrap();
    model.evaluate();
    // The value is untouched; only the content a reader renders moved.
    assert_eq!(
        model.get_cell_value_by_index(0, 1, 1),
        Ok(CellValue::String("abc".to_string()))
    );
    assert_eq!(
        model.get_localized_cell_content(0, 1, 1),
        Ok("'abc".to_string())
    );
    assert_eq!(delta(&mut model), vec![at(1, 1)]);
}

/// Editing a static link is a write like any other, so the cell is listed even
/// with no pass in between.
#[test]
fn attaching_a_link_lists_the_cell_without_an_evaluate() {
    let mut model = tracked_model();
    model
        .set_user_input(0, 1, 1, "IronCalc".to_string())
        .unwrap();
    model.evaluate();
    let _ = delta(&mut model);

    model
        .set_cell_link(
            0,
            1,
            1,
            Link::External {
                target: "http://ironcalc.example".to_string(),
                tooltip: None,
            },
        )
        .unwrap();

    assert_eq!(delta(&mut model), vec![at(1, 1)]);
}

/// Changes that renumber or re-render cells rather than editing them cannot be
/// described by a list of positions, so they report a full repaint.
#[test]
fn changes_no_list_of_cells_describes_report_everything() {
    #[allow(clippy::type_complexity)]
    let changes: Vec<(&str, Box<dyn Fn(&mut Model)>)> = vec![
        (
            "insert_rows",
            Box::new(|m: &mut Model| m.insert_rows(0, 1, 1).unwrap()),
        ),
        (
            "delete_rows",
            Box::new(|m: &mut Model| m.delete_rows(0, 1, 1).unwrap()),
        ),
        (
            "insert_columns",
            Box::new(|m: &mut Model| m.insert_columns(0, 1, 1).unwrap()),
        ),
        (
            "delete_columns",
            Box::new(|m: &mut Model| m.delete_columns(0, 1, 1).unwrap()),
        ),
        (
            "move_rows_action",
            Box::new(|m: &mut Model| m.move_rows_action(0, 1, 1, 3).unwrap()),
        ),
        (
            "move_columns_action",
            Box::new(|m: &mut Model| m.move_columns_action(0, 1, 1, 3).unwrap()),
        ),
        (
            "new_sheet",
            Box::new(|m: &mut Model| {
                m.new_sheet();
            }),
        ),
        (
            "delete_sheet",
            Box::new(|m: &mut Model| {
                m.new_sheet();
                let _ = m.take_changed_cells();
                m.delete_sheet(1).unwrap()
            }),
        ),
        (
            "rename_sheet",
            Box::new(|m: &mut Model| m.rename_sheet_by_index(0, "Renamed").unwrap()),
        ),
        (
            "move_sheet",
            Box::new(|m: &mut Model| {
                m.new_sheet();
                let _ = m.take_changed_cells();
                m.move_sheet(1, 0).unwrap()
            }),
        ),
        (
            "set_locale",
            Box::new(|m: &mut Model| m.set_locale("es").unwrap()),
        ),
        (
            "set_timezone",
            Box::new(|m: &mut Model| m.set_timezone("Europe/Berlin").unwrap()),
        ),
    ];

    for (name, apply) in changes {
        let mut model = tracked_model();
        model.set_user_input(0, 2, 2, "=1+1".to_string()).unwrap();
        model.evaluate();
        let _ = delta(&mut model);

        apply(&mut model);
        model.evaluate();

        assert_eq!(
            model.take_changed_cells(),
            ChangedSinceRead::Everything,
            "{name} should report a full repaint"
        );
    }
}

/// A model that has just been decoded has never been read from, so its reader
/// has painted nothing: the first read is a full repaint, not the empty delta
/// its empty journal would otherwise suggest.
#[test]
fn the_first_read_of_a_model_loaded_from_bytes_is_everything() {
    let mut source = Model::new_empty("m", "en", "UTC", "en").unwrap();
    source.set_user_input(0, 1, 1, "1".to_string()).unwrap();
    source.set_user_input(0, 1, 2, "=A1*2".to_string()).unwrap();
    source.evaluate();
    let bytes = source.to_bytes();

    let mut model = Model::from_bytes(&bytes, "en").unwrap();
    assert_eq!(model.take_changed_cells(), ChangedSinceRead::Everything);

    // And from there it is an ordinary delta.
    model.set_user_input(0, 1, 1, "7".to_string()).unwrap();
    model.evaluate();
    assert_eq!(delta(&mut model), vec![at(1, 1), at(1, 2)]);
}

/// A workbook whose delta nobody reads must not pay for the delta. Tracking
/// snapshots the whole workbook twice per pass, so a tracked pass is markedly
/// more expensive than an untracked one -- and an untracked pass that had been
/// made to snapshot anyway would show the two converging.
#[test]
fn evaluating_a_workbook_nobody_reads_costs_nothing() {
    fn chain_of(rows: i32) -> Model<'static> {
        let mut model = Model::new_empty("m", "en", "UTC", "en").unwrap();
        model.set_user_input(0, 1, 1, "1".to_string()).unwrap();
        for row in 2..=rows {
            model
                .set_user_input(0, row, 1, format!("=A{}+1", row - 1))
                .unwrap();
        }
        model.evaluate();
        model
    }

    fn time(rounds: u32, mut pass: impl FnMut()) -> f64 {
        let start = Instant::now();
        for _ in 0..rounds {
            pass();
        }
        start.elapsed().as_secs_f64() / f64::from(rounds)
    }

    let rounds = 5;
    let mut untracked = chain_of(10_000);
    let mut tracked = chain_of(10_000);
    assert_eq!(tracked.take_changed_cells(), ChangedSinceRead::Everything);
    tracked.evaluate();
    let _ = tracked.take_changed_cells();

    // Warm up: the first pass on each model pays for lazily built state.
    untracked.evaluate();
    tracked.evaluate();
    let _ = tracked.take_changed_cells();

    let untracked_pass = time(rounds, || untracked.evaluate());
    let tracked_pass = time(rounds, || {
        tracked.evaluate();
        let _ = tracked.take_changed_cells();
    });

    println!(
        "10k formulas: untracked {:.1}ms/pass, tracked {:.1}ms/pass, ratio {:.2}",
        untracked_pass * 1e3,
        tracked_pass * 1e3,
        tracked_pass / untracked_pass
    );
    // Measured ratio is ~1.9 in both debug and release; 1.3 leaves room for a
    // noisy machine while still failing outright if the untracked pass starts
    // doing the tracked pass's work.
    assert!(
        tracked_pass > untracked_pass * 1.3,
        "an untracked pass should skip the snapshots: untracked {untracked_pass:.4}s, tracked {tracked_pass:.4}s"
    );
}
