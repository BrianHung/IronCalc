#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

use crate::recalc::Input;
use crate::test::util::{incremental_mode, new_empty_model};
use crate::{ChangedSinceRead, RecalcMode, UserModel};

fn reads_random(model: &crate::Model, p: (u32, i32, i32)) -> bool {
    model.graph.cell_reads(p, |i| matches!(i, Input::Random))
}

fn reads_own_coord(model: &crate::Model, p: (u32, i32, i32)) -> bool {
    model
        .graph
        .cell_reads(p, |i| matches!(i, Input::OwnCoord(_)))
}

fn flush_writes(model: &mut crate::Model) {
    model.drain_write_journal();
}

#[test]
#[test]
fn incremental_handles_row_delete() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "10");
    model._set("A5", "=A1+1");
    model.evaluate();

    // No tracked range straddles the deleted row, so the shift models it and the
    // pass stays incremental.
    model.delete_rows(0, 2, 1).unwrap(); // A5 -> A4, ref A1 intact
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("A4"), "11");
}

#[test]
fn incremental_row_delete_shrinking_range_forces_full() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    for row in 1..=5 {
        model._set(&format!("A{row}"), &row.to_string());
    }
    model._set("C1", "=SUM(A1:A5)");
    model.evaluate();

    // Deleting a row inside the summed range shrinks it; the shift cannot model a
    // partial-range removal, so it falls back to a full recompute rather than
    // leave a stale sum.
    model.delete_rows(0, 3, 1).unwrap(); // drops A3, A1:A5 -> A1:A4
    assert!(model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("C1"), "12"); // 1 + 2 + 4 + 5
}

#[test]
fn incremental_structural_edit_moves_volatile_with_the_graph() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A10", "=RAND()");
    model.evaluate();
    assert!(reads_random(&model, (0, 10, 1)));

    // The graph shifts every position-keyed set, so the Random input travels
    // to A11 and the next pass can stay incremental.
    model.insert_rows(0, 1, 1).unwrap();
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    assert!(reads_random(&model, (0, 11, 1)));
    assert!(!reads_random(&model, (0, 10, 1)));

    model.evaluate();
    assert!(reads_random(&model, (0, 11, 1)));
}

#[test]
fn incremental_insert_moves_hyperlink_with_the_cell() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A10", "=HYPERLINK(\"http://a.com\",\"x\")");
    model.evaluate();
    let dynamic_rows = |model: &crate::Model| {
        model
            .get_links_list(0)
            .unwrap()
            .into_iter()
            .filter(|l| l.dynamic)
            .map(|l| l.row)
            .collect::<Vec<_>>()
    };
    assert_eq!(dynamic_rows(&model), vec![10]);

    model.insert_rows(0, 1, 1).unwrap();
    // The formula map must move with the cell; a leftover at A10 is a ghost.
    assert_eq!(dynamic_rows(&model), vec![11]);
    model.evaluate();
    assert_eq!(dynamic_rows(&model), vec![11]);
}

#[test]
fn incremental_wide_fanout_stays_correct() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    // Exceed INCREMENTAL_FANOUT_FLOOR (1024) so editing A1 trips the fanout
    // guard and falls back to full; the result must still be correct.
    for row in 1..=1100 {
        model._set(&format!("C{row}"), "=$A$1*2");
    }
    model.evaluate();

    model._set("A1", "5");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "10");
    assert_eq!(model._get_text("C1100"), "10");
}

#[test]
fn incremental_defined_name_retarget_forces_full() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    let sheet = model.workbook.worksheets[0].get_name();
    model._set("A1", "10");
    model._set("C1", "777");
    model
        .new_defined_name("myname", None, &format!("{sheet}!$A$1"))
        .unwrap();
    model._set("B1", "=myname");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "10");

    model._set("Z1", "1"); // arms incremental with a pending edit
    model
        .update_defined_name("myname", None, "myname", None, &format!("{sheet}!$C$1"))
        .unwrap(); // reparses and evaluates; must fall back to full
    assert_eq!(model._get_text("B1"), "777"); // not the stale 10
}

#[test]
#[test]
fn incremental_sum_over_offset_sees_updated_target() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "10");
    model._set("D2", "=A1");
    model._set("A3", "=OFFSET(D1,1,0)");
    model._set("C1", "=SUM(A1:A3)");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "20");
    let _ = model.take_changed_cells();

    model._set("A1", "20");
    model.evaluate();
    assert_eq!(model._get_text("D2"), "20");
    assert_eq!(model._get_text("A3"), "20");
    assert_eq!(model._get_text("C1"), "40");
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("expected incremental delta");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(3, 1))); // A3 OFFSET
    assert!(changed.contains(&(1, 3))); // C1 SUM
}

#[test]
fn incremental_running_totals_compose_after_offset_and_insert() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("A2", "2");
    model._set("A3", "3");
    model._set("B1", "=SUM(A1:A1)");
    model._set("B2", "=SUM(A1:A2)");
    model._set("B3", "=SUM(A1:A3)");
    model._set("D2", "=A2");
    model._set("C1", "=OFFSET(D1,1,0)"); // reads D2
    model.evaluate();
    assert_eq!(model._get_text("B1"), "1");
    assert_eq!(model._get_text("B2"), "3");
    assert_eq!(model._get_text("B3"), "6");
    assert_eq!(model._get_text("C1"), "2");

    model._set("A2", "5");
    model.evaluate();
    assert_eq!(model._get_text("B2"), "6");
    assert_eq!(model._get_text("B3"), "9");
    assert_eq!(model._get_text("C1"), "5");

    model.insert_rows(0, 2, 1).unwrap();
    model._set("A2", "4"); // new row must change every prefix that covers it
    model.evaluate();
    assert_eq!(model._get_formula("B4"), "=SUM(A1:A4)");
    assert_eq!(model._get_text("B1"), "1");
    assert_eq!(model._get_text("B3"), "10"); // 1 + 4 + 5
    assert_eq!(model._get_text("B4"), "13"); // 1 + 4 + 5 + 3
                                             // Still OFFSET(D1,1,0); D2 is the new blank, and a blank formula result
                                             // coerces to 0 at the formula boundary, as in Excel and default Full.
    assert_eq!(model._get_text("C1"), "0");
}

#[test]
fn incremental_running_totals_see_offset_inside_range() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("A2", "2");
    model._set("D2", "3");
    model._set("A3", "=OFFSET(D1,1,0)"); // inside the running-total range
    model._set("B1", "=SUM(A1:A1)");
    model._set("B2", "=SUM(A1:A2)");
    model._set("B3", "=SUM(A1:A3)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "1");
    assert_eq!(model._get_text("B2"), "3");
    assert_eq!(model._get_text("B3"), "6");
    let _ = model.take_changed_cells();

    model._set("D2", "10"); // OFFSET target; A3 and B3 must not read a stale memo
    model.evaluate();
    assert_eq!(model._get_text("A3"), "10");
    assert_eq!(model._get_text("B1"), "1");
    assert_eq!(model._get_text("B2"), "3");
    assert_eq!(model._get_text("B3"), "13");
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("expected incremental delta");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(3, 1))); // A3
    assert!(changed.contains(&(3, 2))); // B3
}

#[test]
fn incremental_set_locale_forces_full() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1234.5");
    model._set("B1", "=TEXT(A1,\"#,##0.00\")");
    model._set("Z1", "0"); // independent
    model.evaluate();
    assert_eq!(model._get_text("B1"), "1,234.50");

    model._set("Z1", "1"); // arms incremental with a pending edit
    model.set_locale("de").unwrap(); // evaluates internally; must force full
    assert_eq!(model._get_text("B1"), "1.234,50"); // not the stale en format
}

#[test]
fn incremental_tracks_dynamic_branch_dependencies() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("D1", "1");
    model._set("E1", "10");
    model._set("F1", "20");
    model._set("A1", "=IF(D1>0,E1,F1)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "10"); // reads E1

    model._set("D1", "-1"); // flip: A1 now reads F1
    model.evaluate();
    assert_eq!(model._get_text("A1"), "20");

    model._set("F1", "99"); // edit the newly-read cell
    model.evaluate();
    assert_eq!(model._get_text("A1"), "99");
}

/// A blocked spill (`#SPILL!`) stores `r=(1,1)`. An insert below it still
/// resets the anchor so the next pass can re-try the spill. That reset must
/// journal even though the formula index is unchanged (fuzz seed 51).
#[test]
fn blocked_spill_reset_on_insert_below_matches_full() {
    let mut inc = new_empty_model().with_recalc_mode(incremental_mode());
    inc._set("A1", "1");
    inc._set("A2", "2");
    inc._set("A3", "3");
    inc._set("A4", "4");
    inc._set("B1", "4");
    inc._set("B2", "3");
    inc._set("B3", "2");
    inc._set("B4", "1");
    inc._set("E17", "=SORTBY(A1:A4,B1:B4)");
    inc._set("E18", "=FILTER(A1:A4,B1:B4>0)");
    inc.evaluate();
    inc.insert_rows(0, 21, 1).unwrap();
    inc.evaluate();

    let mut full = new_empty_model();
    full._set("A1", "1");
    full._set("A2", "2");
    full._set("A3", "3");
    full._set("A4", "4");
    full._set("B1", "4");
    full._set("B2", "3");
    full._set("B3", "2");
    full._set("B4", "1");
    full._set("E17", "=SORTBY(A1:A4,B1:B4)");
    full._set("E18", "=FILTER(A1:A4,B1:B4>0)");
    full.evaluate();
    full.insert_rows(0, 21, 1).unwrap();
    full.evaluate();

    assert_eq!(inc._get_text("E17"), full._get_text("E17"));
    assert_eq!(
        inc._get_cell("E17").get_type(),
        full._get_cell("E17").get_type()
    );
}

#[test]
fn incremental_undo_under_pause_stays_correct() {
    let mut model = UserModel::new_empty("m", "en", "UTC", "en")
        .unwrap()
        .with_recalc_mode(incremental_mode());
    model.set_user_input(0, 1, 1, "1").unwrap(); // A1
    model.set_user_input(0, 1, 2, "=A1+1").unwrap(); // B1 = 2
    model.set_user_input(0, 1, 3, "10").unwrap(); // C1 (independent)
    model.set_user_input(0, 1, 4, "=C1+1").unwrap(); // D1
    model.set_user_input(0, 1, 1, "5").unwrap(); // A1 = 5 -> B1 = 6

    model.pause_evaluation();
    model.undo().unwrap(); // A1 back to 1, evaluation deferred
    model.set_user_input(0, 1, 3, "20").unwrap(); // unrelated edit re-arms incremental
    model.resume_evaluation();
    model.evaluate();
    assert_eq!(model.get_formatted_cell_value(0, 1, 2).unwrap(), "2"); // B1 repaired, not stale 6
    assert_eq!(model.get_formatted_cell_value(0, 1, 4).unwrap(), "21"); // D1
}

#[test]
fn take_changed_cells_reports_incremental_delta() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("B1", "=A1*2");
    model._set("C1", "100"); // independent
    model.evaluate();
    // Switching mode forces a full pass, so everything is potentially changed.
    assert_eq!(model.take_changed_cells(), ChangedSinceRead::Everything);

    model._set("A1", "5"); // A1 and its dependent B1 recompute; C1 untouched
    model.evaluate();
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("expected incremental delta");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(1, 1))); // A1 edited
    assert!(changed.contains(&(1, 2))); // B1 recomputed
    assert!(!changed.contains(&(1, 3))); // C1 not touched

    assert_eq!(model.take_changed_cells(), ChangedSinceRead::Cells(vec![])); // reading clears

    model._set("D1", "=A1+1"); // a new formula records its edges on first evaluate
    model.evaluate();
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("a new scalar formula must stay Incremental");
    };
    assert!(cells.iter().any(|c| c.row == 1 && c.column == 4));
}

#[test]
fn take_changed_cells_reports_everything_for_data_only_shift() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "10");
    model._set("A2", "20");
    model._set("Z1", "1");
    model.evaluate();
    let _ = model.take_changed_cells();

    // Insert above two data cells. Nothing is a formula dependent, so a cell
    // list would be empty while A1/A2 visibly moved.
    model.insert_rows(0, 1, 1).unwrap();
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model.take_changed_cells(), ChangedSinceRead::Everything);
    assert_eq!(model._get_text("A2"), "10");
    assert_eq!(model._get_text("A3"), "20");

    // The flag dies with the pass. A later value edit is a cell list again.
    model._set("Z1", "2");
    model.evaluate();
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("structural Everything must not leak into the next pass");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(1, 26)));
}

#[test]
fn take_changed_cells_reports_everything_for_trailing_delete() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "10");
    model.evaluate();
    let _ = model.take_changed_cells();

    // Last populated row, nothing below, no formulas. Dirty stays empty so
    // evaluate would take the Full keep-delta branch; the emptied cell must
    // still be Everything, not Cells([]).
    model.delete_rows(0, 1, 1).unwrap();
    // Ready + empty dirty: the Full keep-delta branch, not MustRebuild.
    assert!(model.graph.should_recompute_full());
    assert!(!model.graph.full_reflects_change());
    model.evaluate();
    assert_eq!(model.take_changed_cells(), ChangedSinceRead::Everything);
    assert_eq!(model._get_text("A1"), "");
}

#[test]
fn take_changed_cells_survives_redundant_evaluate() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("B1", "=A1*2");
    model.evaluate();
    let _ = model.take_changed_cells(); // clear the mode-switch full

    model._set("A1", "5");
    model.evaluate(); // incremental: records A1, B1
    model.evaluate(); // redundant no-op full: must keep the delta

    // The delta survives the redundant evaluate, so this is Cells, not Everything.
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("expected incremental delta");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(1, 1)));
    assert!(changed.contains(&(1, 2)));
}

#[test]
fn incremental_reports_dynamic_link_retarget() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "http://a.com");
    model._set("B1", "=HYPERLINK(A1,\"click\")"); // label "click", target A1
    model.evaluate();
    let _ = model.take_changed_cells();

    model._set("A1", "http://b.com"); // label unchanged, only the target moves
    model.evaluate();
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("expected incremental delta");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(1, 2))); // B1's link changed, so it is reported
}

#[test]
fn incremental_reports_conditional_format_change() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "5");
    model._set("B1", "x");
    model
        .add_conditional_formatting(
            0,
            "B1",
            crate::cf_types::CfRuleInput::Formula {
                formula: "=$A$1>10".to_string(),
                format: crate::types::Dxf::default(),
                stop_if_true: false,
            },
        )
        .unwrap();
    model.evaluate();
    let _ = model.take_changed_cells();

    model._set("A1", "20"); // flips B1's rule on, though B1's value is unchanged
    model.evaluate();
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("expected incremental delta");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(1, 2))); // B1's conditional format changed
}

#[test]
fn incremental_reports_cf_only_mutation() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "5");
    model._set("B1", "x");
    model.evaluate();
    let _ = model.take_changed_cells();

    model
        .add_conditional_formatting(
            0,
            "B1",
            crate::cf_types::CfRuleInput::Formula {
                formula: "=$A$1>3".to_string(),
                format: crate::types::Dxf::default(),
                stop_if_true: false,
            },
        )
        .unwrap();
    model.evaluate(); // a CF-rule edit with no cell value change
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("expected incremental delta");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(1, 2))); // B1's new format is reported
}

#[test]
fn incremental_reports_signed_zero_flip() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "5");
    model._set("B1", "=A1*0");
    model._set("C1", "=B1&\"!\"");
    model.evaluate();
    let _ = model.take_changed_cells();

    model._set("A1", "-5"); // B1: +0 -> -0, observable in C1's text
    model.evaluate();
    assert_eq!(model._get_text("C1"), "-0!"); // not the stale "0!"
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("expected incremental delta");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(1, 3))); // C1 reported
}

#[test]
fn incremental_reports_only_value_changes() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("B1", "=A1*0"); // always 0
    model._set("C1", "=B1+1"); // always 1
    model.evaluate();
    let _ = model.take_changed_cells(); // clear the mode-switch full

    model._set("A1", "5");
    model.evaluate();
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("expected incremental delta");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(1, 1))); // A1 moved 1 -> 5
    assert!(!changed.contains(&(1, 2))); // B1 recomputed but stayed 0
    assert!(!changed.contains(&(1, 3))); // C1 not reached
    assert_eq!(model._get_text("C1"), "1"); // still correct
}

#[test]
fn incremental_propagates_error_to_text_transition() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1"); // truthy: take the error branch
    model._set("B1", "=IF(A1, 1/0, \"#DIV/0!\")"); // error #DIV/0!
    model._set("C1", "=ISERROR(B1)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "#DIV/0!");
    assert_eq!(model._get_text("C1"), "TRUE");

    let _ = model.take_changed_cells();
    model._set("A1", "0"); // falsy: B1 becomes the literal text "#DIV/0!"
    model.evaluate();
    assert_eq!(model._get_text("B1"), "#DIV/0!"); // same string, now text
    assert_eq!(model._get_text("C1"), "FALSE"); // dependent must flip
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("expected incremental delta");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(1, 2))); // B1 type-only
    assert!(changed.contains(&(1, 3))); // C1 dependent
}

#[test]
fn range_composition_does_not_memoize_transient_circ() {
    // A2's SUM sees A2 mid-cycle (#CIRC → IFERROR → 5). That must not be
    // cached for B1, which should see A2 after it settles.
    for incremental in [false, true] {
        let mut model = if incremental {
            new_empty_model().with_recalc_mode(incremental_mode())
        } else {
            new_empty_model()
        };
        model._set("A1", "1");
        model._set("A2", "=IFERROR(SUM(A1:A2),5)");
        model._set("B1", "=SUM(A1:A2)");
        model.evaluate();
        assert_eq!(model._get_text("A2"), "5", "incremental={incremental}");
        assert_eq!(model._get_text("B1"), "6", "incremental={incremental}");
    }
}

#[test]
fn full_mode_sum_matches_precomposition_association() {
    // IRONCALC_RECALC=verify would compose per-row (0.0). This asserts Full isolation.
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Full);
    model.update_cell_with_number(0, 1, 1, 1e16).unwrap();
    model.update_cell_with_number(0, 1, 2, 1.0).unwrap();
    model.update_cell_with_number(0, 2, 1, -1e16).unwrap();
    model.update_cell_with_number(0, 2, 2, 1.0).unwrap();
    model._set("C1", "=SUM(A1:B2)");
    model.evaluate();
    // Row-major 1e16+1-1e16+1 = 1. Per-row composition yields 0.
    assert_eq!(
        model.get_cell_value_by_index(0, 1, 3).unwrap(),
        crate::cell::CellValue::Number(1.0)
    );
}

#[test]
fn range_composition_does_not_memoize_transient_count_circ() {
    for incremental in [false, true] {
        let mut model = if incremental {
            new_empty_model().with_recalc_mode(incremental_mode())
        } else {
            new_empty_model()
        };
        model._set("A1", "1");
        model._set("A2", "=COUNT(A1:A2)");
        model._set("B1", "=COUNT(A1:A2)");
        model.evaluate();
        assert_eq!(model._get_text("A2"), "1", "incremental={incremental}");
        assert_eq!(model._get_text("B1"), "2", "incremental={incremental}");
    }
}

#[test]
fn incremental_sumifs_reads_resized_criteria() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("A2", "1");
    model._set("A3", "1");
    model._set("B1", "1");
    model._set("B2", "1");
    model._set("B3", "1");
    model._set("C1", "=SUMIFS(A1:A3,B1:B1,\">0\")");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "3");

    model._set("B2", "0");
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("C1"), "2");
}

#[test]
fn incremental_argless_row_updates_after_insert() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "=ROW()");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1");
    assert!(reads_own_coord(&model, (0, 1, 1)));

    model.insert_rows(0, 1, 1).unwrap();
    model.evaluate();
    assert_eq!(model._get_text("A2"), "2");
}

#[test]
fn incremental_subtotal_sees_hidden_row() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("A2", "2");
    model._set("A3", "3");
    model._set("B1", "=SUBTOTAL(109,A1:A3)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "6");

    model.set_row_hidden(0, 2, true).unwrap();
    model._set("D9", "1");
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("B1"), "4");
}

#[test]
fn incremental_overwrite_spill_anchor_updates_dependents() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "=SEQUENCE(3)");
    model._set("C2", "=A2*10");
    model.evaluate();
    assert_eq!(model._get_text("A2"), "2");
    assert_eq!(model._get_text("C2"), "20");

    // Keep the anchor in `arrays` so this pass Full-falls-back; clearing that
    // role would stay Incremental and leave C2 stale after the spill is wiped.
    model._set("A1", "5");
    assert!(model.graph.arrays.contains(&(0, 1, 1)));
    model.evaluate();
    assert_eq!(model._get_text("A1"), "5");
    assert_eq!(model._get_text("A2"), "");
    assert_eq!(model._get_text("A3"), "");
    assert_eq!(model._get_text("C2"), "0");
    assert!(!model.graph.arrays.contains(&(0, 1, 1)));
}

#[test]
#[test]
fn incremental_spill_invalidates_composed_range_cache() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "=SEQUENCE(2,1,B1)");
    model._set("B1", "=SUM(A6:A7)");
    model._set("A5", "=SEQUENCE(3)");
    model._set("A9", "=SUM(A6:A7)");
    model.evaluate();
    assert_eq!(model._get_text("A9"), "5");
}

#[test]
fn min_max_signed_zero_matches_full() {
    // The ±0 tie-break of `f64::min`/`max` is platform-defined (LLVM folds
    // constants to +0 on x86_64, runtime minsd picks the first operand), so no
    // fixed bit pattern is portable. What must hold everywhere is mode parity:
    // Incremental's composed MIN/MAX returns the same bits as Full's direct
    // row-major scan on the platform it runs on.
    let run = |mode: crate::RecalcMode| {
        let mut model = new_empty_model().with_recalc_mode(mode);
        model.update_cell_with_number(0, 1, 1, 0.0).unwrap();
        model.update_cell_with_number(0, 2, 1, -0.0).unwrap();
        model._set("A3", "=MIN(A1:A2)");
        model._set("A4", "=MAX(A1:A2)");
        model.evaluate();
        let bits = |row: i32| match model.get_cell_value_by_index(0, row, 1).unwrap() {
            crate::cell::CellValue::Number(n) => n.to_bits(),
            other => panic!("expected number, got {other:?}"),
        };
        (bits(3), bits(4))
    };
    assert_eq!(
        run(crate::RecalcMode::Full),
        run(crate::RecalcMode::Incremental)
    );
}

#[test]
fn stored_empty_formula_is_live_zero() {
    // Excel coerces a blank formula result to 0 at the result boundary
    // (cached `<v>0</v>`), and the store must agree with what same-pass and
    // out-of-cone readers see, in every mode.
    let run = |mode: crate::RecalcMode| {
        let mut model = new_empty_model().with_recalc_mode(mode);
        model._set("B1", "=A1");
        model.evaluate();
        (
            model._get_text("B1"),
            model.get_cell_value_by_index(0, 1, 2).unwrap(),
        )
    };
    assert_eq!(run(crate::RecalcMode::Full).0, "0");
    assert_eq!(
        run(crate::RecalcMode::Incremental),
        run(crate::RecalcMode::Full)
    );
}

#[test]
fn offset_target_change_without_static_edge() {
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    model._set("C1", "0");
    model._set("D1", "1");
    model._set("E1", "2");
    model._set("A1", "=OFFSET(D1,0,C1)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1");
    let _ = model.take_changed_cells();

    model._set("E1", "99");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1");
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => panic!("unrelated E1 edit must stay Incremental"),
        ChangedSinceRead::Cells(cells) => {
            let changed: std::collections::HashSet<(i32, i32)> =
                cells.iter().map(|c| (c.row, c.column)).collect();
            assert!(changed.contains(&(1, 5)));
            assert!(!changed.contains(&(1, 1)));
        }
    }

    model._set("C1", "1");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "99");
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => panic!("OFFSET retarget must stay Incremental"),
        ChangedSinceRead::Cells(cells) => {
            let changed: std::collections::HashSet<(i32, i32)> =
                cells.iter().map(|c| (c.row, c.column)).collect();
            assert!(changed.contains(&(1, 1)));
        }
    }
}

#[test]
fn subtotal_sees_hidden_row_it_scans_not_own_row() {
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    for row in 1..=10 {
        model._set(&format!("A{row}"), "1");
    }
    model._set("A50", "=SUBTOTAL(109,A1:A10)");
    model.evaluate();
    assert_eq!(model._get_text("A50"), "10");
    let _ = model.take_changed_cells();

    model.set_row_hidden(0, 5, true).unwrap();
    model.evaluate();
    assert_eq!(model._get_text("A50"), "9");
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => panic!("hide scanned row must stay Incremental"),
        ChangedSinceRead::Cells(cells) => {
            assert!(cells.iter().any(|c| c.row == 50 && c.column == 1));
        }
    }

    model.set_row_hidden(0, 50, true).unwrap();
    model.evaluate();
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => panic!("hide own row must not force Full"),
        ChangedSinceRead::Cells(cells) => {
            assert!(
                !cells.iter().any(|c| c.row == 50 && c.column == 1),
                "SUBTOTAL must not recompute when its own row is hidden"
            );
        }
    }
    assert_eq!(model._get_text("A50"), "9");
}

#[test]
fn name_reader_redirty_on_insert() {
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    let sheet = model.workbook.worksheets[0].get_name();
    model._set("A10", "42");
    model
        .new_defined_name("MyRef", None, &format!("{sheet}!$A$10"))
        .unwrap();
    model._set("B1", "=MyRef");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "42");

    model.insert_rows(0, 5, 1).unwrap();
    model.evaluate();
    // The name now reads a blank cell: a blank formula result coerces to 0
    // (Excel parity), not "".
    assert_eq!(model._get_text("B1"), "0");

    model._set("A10", "99");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "99");
}

#[test]
fn redundant_evaluate_keeps_rand_reporting_but_not_sumifs() {
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    model._set("A1", "1");
    model._set("B1", "=SUMIFS(A1:A1,A1:A1,\">0\")");
    model._set("C1", "=RAND()");
    model.evaluate();
    let _ = model.take_changed_cells();

    model._set("A1", "5");
    model.evaluate();
    let _ = model.take_changed_cells();

    model.evaluate();
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => {}
        other => panic!("RAND must keep reporting Everything, got {other:?}"),
    }

    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    model._set("A1", "1");
    model._set("B1", "=SUMIFS(A1:A1,A1:A1,\">0\")");
    model.evaluate();
    let _ = model.take_changed_cells();
    model._set("A1", "5");
    model.evaluate();
    let _ = model.take_changed_cells();
    model.evaluate();
    match model.take_changed_cells() {
        ChangedSinceRead::Cells(cells) => assert!(
            cells.is_empty(),
            "SUMIFS must not always-report on a redundant evaluate"
        ),
        ChangedSinceRead::Everything => {
            panic!("SUMIFS must not wipe the delta on a redundant evaluate")
        }
    }
}

/// SUBTOTAL records FormulaText of scanned cells. A formula write in that
/// range re-evals SUBTOTAL but must not always-report it when the aggregate
/// is unchanged (fuzz seed 31).
#[test]
fn subtotal_formula_text_reread_is_not_always_reported() {
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    model._set("A1", "1");
    model._set("B1", "=SUBTOTAL(9,A1:A5)");
    model.evaluate();
    let _ = model.take_changed_cells();

    model._set("A2", "=0");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "1");
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => panic!("SUBTOTAL reread must stay Incremental"),
        ChangedSinceRead::Cells(cells) => {
            let changed: std::collections::HashSet<(i32, i32)> =
                cells.iter().map(|c| (c.row, c.column)).collect();
            assert!(
                changed.contains(&(2, 1)),
                "A2 is a write seed, got {changed:?}"
            );
            assert!(
                !changed.contains(&(1, 2)),
                "SUBTOTAL must not always-report when the aggregate is unchanged, got {changed:?}"
            );
        }
    }
}

#[test]
fn journal_value_over_formula_drops_edges() {
    for overwrite in [
        |model: &mut crate::Model| model.update_cell_with_number(0, 1, 1, 5.0).unwrap(),
        |model: &mut crate::Model| model.update_cell_with_text(0, 1, 1, "5").unwrap(),
        |model: &mut crate::Model| model.update_cell_with_bool(0, 1, 1, true).unwrap(),
        |model: &mut crate::Model| {
            model.set_user_input(0, 1, 1, "5".to_string()).unwrap();
        },
    ] {
        let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
        model._set("A1", "=RAND()");
        model.evaluate();
        overwrite(&mut model);
        model.evaluate();
        let _ = model.take_changed_cells();
        model._set("B1", "1");
        model.evaluate();
        model._set("B1", "2");
        model.evaluate();
        match model.take_changed_cells() {
            ChangedSinceRead::Everything => panic!("overwritten RAND must not force Everything"),
            ChangedSinceRead::Cells(cells) => {
                assert!(
                    !cells.iter().any(|c| c.row == 1 && c.column == 1),
                    "overwritten RAND must not stay in later deltas"
                );
            }
        }
    }
}

#[test]
fn formulatext_sees_value_overwrite_of_its_argument() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "=1+1");
    model._set("B1", "=FORMULATEXT(A1)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "=1+1");

    model._set("A1", "ov");
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("B1"), "#N/A");
}

#[test]
fn isformula_sees_value_overwrite_of_its_argument() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "=1+1");
    model._set("B1", "=ISFORMULA(A1)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "TRUE");

    model.update_cell_with_number(0, 1, 1, 36.0).unwrap();
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("B1"), "FALSE");
}

#[test]
fn style_on_blank_cell_does_not_enter_the_delta() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model.evaluate();
    let _ = model.take_changed_cells();
    let mut style = model.get_style_for_cell(0, 13, 3).unwrap();
    style.fill.color = crate::types::Color::Rgb("#FFAA00".to_string());
    model.set_cell_style(0, 13, 3, &style).unwrap();
    model.evaluate();
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => panic!("style on a blank cell must stay Incremental"),
        ChangedSinceRead::Cells(cells) => assert!(
            cells.is_empty(),
            "style on a blank cell must not appear in the delta, got {cells:?}"
        ),
    }
}

/// I2.5 — a batch's pre-batch formula-ness is read off its *first* entry.
///
/// Between full passes `formula_cell_count` is maintained by the journal
/// rather than recounted, and the drain accounts each cell once: the batch's
/// final state against the state it held before the batch began. Only the
/// first entry carries that; every later entry's `was_formula` is an
/// intermediate the batch already passed through.
///
/// Kills `first_was.entry(p).or_insert(..)` becoming `insert(..)` (last-wins),
/// in both directions — a formula round-tripped through a value would count a
/// formula that was already there, and a value round-tripped through a formula
/// would discount one that never existed. The counter is checked against a
/// from-scratch recount rather than a literal, so the test states the
/// invariant and not an arithmetic coincidence.
#[test]
fn journal_accounts_a_batch_against_its_pre_batch_state() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "=1+1");
    model._set("B1", "7");
    model.evaluate();

    // Each direction is checked on its own: batched together they would be a
    // double-count and a double-discount that cancel, and the drain would look
    // right while being wrong twice.
    fn agrees(model: &mut crate::Model, label: &str) {
        flush_writes(model);
        let from_journal = model.formula_cell_count;
        model.recount_formula_cells();
        assert_eq!(
            from_journal, model.formula_cell_count,
            "journal-maintained formula count disagrees with a recount after {label}"
        );
    }

    // A formula round-tripped through a value is still the formula it was.
    model._set("A1", "5");
    model._set("A1", "=2+2");
    agrees(&mut model, "formula -> value -> formula");

    // A value round-tripped through a formula never became one.
    model._set("B1", "=3");
    model._set("B1", "9");
    agrees(&mut model, "value -> formula -> value");
}

#[test]
fn journal_rejected_write_logs_nothing() {
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    model._set("A1", "1");
    model.evaluate();
    let _ = model.take_changed_cells();
    assert!(model.set_user_input(1, 9, 3, "84".to_string()).is_err());
    model.evaluate();
    match model.take_changed_cells() {
        ChangedSinceRead::Cells(cells) => assert!(cells.is_empty()),
        ChangedSinceRead::Everything => panic!("rejected write must not force Everything"),
    }
}

/// `E15#` of an empty cell is `#REF!` until E15 is written. The `#` operator
/// must record a cell edge on the empty anchor so the later write re-runs it
/// (fuzz seed 3, 200 steps).
#[test]
fn spill_hash_of_empty_anchor_sees_later_formula() {
    let mut inc = new_empty_model().with_recalc_mode(incremental_mode());
    inc._set("E16", "=E15#");
    inc.evaluate();
    inc._set("E15", "=ROWS(A1:A8)");
    inc.evaluate();

    let mut full = new_empty_model();
    full._set("E16", "=E15#");
    full.evaluate();
    full._set("E15", "=ROWS(A1:A8)");
    full.evaluate();

    assert_eq!(inc._get_text("E16"), full._get_text("E16"));
    assert_eq!(inc._get_text("E16"), "8");
}

/// The column move rebuilds the moved column one cell at a time, in row order
/// (`column_cell_references` sorts), so the anchor of a CSE array is written
/// back before the placeholders of the rectangle it re-declares. That is an
/// interim state of the move, not a user write: `move_column_unchecked` must
/// suspend the CSE member guard around the rebuild (the way `move_cell` does)
/// or the placeholder writes error and the move fails. The mode does not
/// matter -- the rebuild happens before any recalc -- so Full alone is pinned.
#[test]
fn moving_a_column_with_a_cse_anchor_always_succeeds() {
    let mut model = new_empty_model().with_recalc_mode(RecalcMode::Full);
    model
        .set_user_array_formula(0, 1, 5, 1, 2, "=B1:B2")
        .unwrap();
    model.move_columns_action(0, 5, 1, 2).unwrap();
    model.evaluate();
    assert_eq!(model._get_formula("G1"), "=B1:B2");
}

/// The same rebuild along the other axis: `move_row_unchecked` writes the
/// anchor and then the placeholders of a horizontal rectangle, in column
/// order, and must suspend the guard the same way.
#[test]
fn moving_a_row_with_a_cse_anchor_always_succeeds() {
    let mut model = new_empty_model().with_recalc_mode(RecalcMode::Full);
    model
        .set_user_array_formula(0, 5, 1, 2, 1, "=A1:B1")
        .unwrap();
    model.move_rows_action(0, 5, 1, 2).unwrap();
    model.evaluate();
    assert_eq!(model._get_formula("A7"), "=A1:B1");
}

/// A read of a multi-column rectangle must be recorded as a rectangle, not
/// dropped. `SUM(B:C)` clips its per-cell walk to the used range, so the only
/// edge that can connect a write below the last used row to the sum is the
/// recorded rect. Dropping wide rects from the read set leaves A1 stale.
#[test]
fn multi_column_range_edits_propagate() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("B1", "1");
    model._set("A1", "=SUM(B:C)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1");

    // Second column of the rect, past the used range: only the rectangle
    // connects this write to A1.
    model._set("C100", "5");
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("A1"), "6");

    // ... and the first column of the same rect.
    model._set("B50", "4");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "10");
}

/// A volatile cell must re-roll on every incremental pass, not only on a full
/// one: it stays in the always-dirty set, which seeds every cone and every
/// delta. Unrelated value edits keep the passes incremental; RANDBETWEEN has
/// to move anyway, its dependent has to follow, and the delta has to name it
/// on every pass. Several passes, so one unlucky equal draw cannot flake.
#[test]
fn volatile_rerolls_across_repeated_incremental_passes() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "=RANDBETWEEN(1,1000000000)");
    model._set("A2", "=A1+0"); // a dependent: the re-roll must propagate
    model._set("B2", "0");
    model.evaluate();
    let _ = model.take_changed_cells();

    let first = model.get_cell_value_by_index(0, 1, 1).unwrap();
    let first_dependent = model.get_cell_value_by_index(0, 2, 1).unwrap();
    let mut rerolled = false;
    let mut dependent_rerolled = false;
    for pass in 1..=5 {
        model._set("B2", &pass.to_string());
        flush_writes(&mut model);
        assert!(!model.graph.should_recompute_full());
        model.evaluate();
        if model.get_cell_value_by_index(0, 1, 1).unwrap() != first {
            rerolled = true;
        }
        if model.get_cell_value_by_index(0, 2, 1).unwrap() != first_dependent {
            dependent_rerolled = true;
        }
        let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
            panic!("an unrelated value edit must stay incremental");
        };
        assert!(
            cells
                .iter()
                .any(|c| (c.sheet, c.row, c.column) == (0, 1, 1)),
            "the delta must name the volatile cell on every pass: {cells:?}"
        );
    }
    assert!(rerolled, "RANDBETWEEN never re-rolled over five passes");
    assert!(
        dependent_rerolled,
        "the volatile's dependent never followed the re-roll"
    );
}

/// NOW() is volatile too, and the clock is mocked in the test build, so its
/// value cannot be used to detect a re-roll. What is observable is that it
/// stays in the always-dirty set and is reported on every incremental pass.
#[test]
fn clock_volatile_is_reported_on_every_incremental_pass() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "=NOW()");
    model._set("A2", "=TODAY()");
    model._set("B2", "0");
    model.evaluate();
    let _ = model.take_changed_cells();

    for pass in 1..=3 {
        model._set("B2", &pass.to_string());
        flush_writes(&mut model);
        assert!(!model.graph.should_recompute_full());
        model.evaluate();
        let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
            panic!("an unrelated value edit must stay incremental");
        };
        for position in [(0, 1, 1), (0, 2, 1)] {
            assert!(
                cells.iter().any(|c| (c.sheet, c.row, c.column) == position),
                "the delta must name the clock volatile {position:?}: {cells:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Convergence: incremental must equal full after *every* evaluate, including
// the passes where full heals debt its own previous pass left behind. Each
// test below drives both modes in lockstep over a minimized shape from the
// differential fuzzer and compares the whole workbook after each evaluate.
// ---------------------------------------------------------------------------

/// Every populated cell's value, for a mode-vs-mode comparison.
fn workbook_values(model: &crate::Model) -> Vec<(u32, i32, i32, String)> {
    let mut cells: Vec<(u32, i32, i32, String)> = model
        .get_all_cells()
        .into_iter()
        .map(|c| {
            (
                c.index,
                c.row,
                c.column,
                model
                    .get_formatted_cell_value(c.index, c.row, c.column)
                    .unwrap_or_default(),
            )
        })
        .collect();
    cells.sort();
    cells
}

fn assert_same_workbook(full: &crate::Model, incremental: &crate::Model, label: &str) {
    assert_eq!(
        workbook_values(full),
        workbook_values(incremental),
        "incremental diverged from full after {label}"
    );
}

/// (a) A row move forces a full pass, but that pass leaves spill debt: `G12`
/// read `E16:E17` before `E15`'s `SEQUENCE` refilled them. Full heals on its
/// next unconditional pass; incremental used to serve `G12` its stored 0 for
/// ever, because no later cone ever named it.
#[test]
fn incremental_heals_spill_debt_left_by_a_forced_full_pass() {
    let build = |mode| {
        let mut m = new_empty_model().with_recalc_mode(mode);
        m._set("E15", "=SEQUENCE(3)");
        m._set("G12", "=SUM(E19:E20)");
        m._set("F14", "=SEQUENCE(3)+G12");
        m
    };
    let mut full = build(crate::RecalcMode::Full);
    let mut inc = build(incremental_mode());
    for m in [&mut full, &mut inc] {
        m.evaluate();
        m.move_rows_action(0, 19, 2, -3).unwrap();
        m.evaluate();
    }
    assert_same_workbook(&full, &inc, "the move's own full pass");
    // The healing pass. The edit is deliberately unrelated to the spill: the
    // dirty cone cannot reach G12, so only the debt signal brings it back.
    for m in [&mut full, &mut inc] {
        m._set("B5", "1");
        m.evaluate();
    }
    assert_eq!(full._get_text("G12"), "5");
    assert_same_workbook(&full, &inc, "the healing pass");
}

/// (b) An error-absorbing function makes the divergence a value, not just a
/// placement: the frontier evaluated `B1` once through `A1`'s recursion (the
/// mid-cycle value full keeps) and then a second time at `B1`'s own topological
/// slot, against the settled `A1`. Full evaluates each cell once per pass.
#[test]
fn incremental_does_not_re_evaluate_a_mid_cycle_cell() {
    let build = |mode| {
        let mut m = new_empty_model().with_recalc_mode(mode);
        m._set("A1", "1");
        m._set("B1", "=IFERROR(A1+1,50)");
        m
    };
    let mut full = build(crate::RecalcMode::Full);
    let mut inc = build(incremental_mode());
    for m in [&mut full, &mut inc] {
        m.evaluate();
        m._set("A1", "=B1");
        m.evaluate();
    }
    assert_eq!(full._get_text("A1"), "50");
    assert_eq!(full._get_text("B1"), "50");
    assert_same_workbook(&full, &inc, "the IFERROR cycle pass");
}

/// A blocked anchor stores `#SPILL!`, but a reader that reaches it while it is
/// still evaluating gets the live array's top-left instead -- here `B1`, an
/// anchor of full's phase 1, pulls `A7` in ahead of `E15`, so full's `A7` holds
/// `1 + D1` and not `-1 + D1`. That value is not a function of the store: a
/// later cone that names `A7` without naming `E15` would recompute it against
/// the stored error and land on `-1 + D1` for ever. Only the full pass
/// evaluates the anchor live, so a cone reaching a blocked anchor's reader has
/// to fall back to one.
#[test]
fn a_blocked_anchors_reader_is_recomputed_only_by_a_full_pass() {
    let build = |mode| {
        let mut m = new_empty_model().with_recalc_mode(mode);
        m._set("E15", "=SEQUENCE(3)");
        // E17 blocks the spill, so E15 stores #SPILL!.
        m._set("E17", "7");
        m._set("D1", "1");
        m._set("A7", "=IFERROR(E15,-1)+D1");
        // A phase-1 anchor above A7: full evaluates it, and so A7, before it
        // reaches E15 in its own right.
        m._set("B1", "=SEQUENCE(1,1,A7,1)");
        m
    };
    let mut full = build(crate::RecalcMode::Full);
    let mut inc = build(incremental_mode());
    for m in [&mut full, &mut inc] {
        m.evaluate();
    }
    assert_eq!(full._get_text("E15"), "#SPILL!");
    assert_eq!(full._get_text("A7"), "2");
    assert_same_workbook(&full, &inc, "the first evaluate");
    // A plain value edit whose cone names A7 but not E15.
    for m in [&mut full, &mut inc] {
        m._set("D1", "2");
        m.evaluate();
    }
    assert_eq!(full._get_text("A7"), "3");
    assert_same_workbook(&full, &inc, "an edit reaching a blocked anchor's reader");
}

/// The audit of the acyclic path: `recompute_frontier` orders by edges, not by
/// position, so a scalar anchor's readers are already ordered after it and the
/// phase-1 gap cannot bite -- there is no cycle for a walk order to break, and
/// a dependency-respecting order is unique in what it produces. Here the anchor
/// sits *below* its reader in row-major order, so a one-phase positional walk
/// would be wrong and the topological one is right.
#[test]
fn acyclic_cone_orders_a_scalar_anchor_by_edges_not_position() {
    let build = |mode| {
        let mut m = new_empty_model().with_recalc_mode(mode);
        m._set("D1", "1");
        // A1 (walked first, row-major) reads B9, the anchor (walked last).
        m._set("A1", "=B9+1");
        m._set("B9", "=SEQUENCE(1,1,D1*10,1)");
        m
    };
    let mut full = build(crate::RecalcMode::Full);
    let mut inc = build(incremental_mode());
    for m in [&mut full, &mut inc] {
        m.evaluate();
    }
    assert_same_workbook(&full, &inc, "the first evaluate");
    for value in ["2", "3"] {
        for m in [&mut full, &mut inc] {
            m._set("D1", value);
            m.evaluate();
        }
        assert_same_workbook(&full, &inc, "an acyclic edit reaching a scalar anchor");
    }
    assert_eq!(full._get_text("A1"), "31");
}

/// Verify's liveness check asserts against the always-dirty set as it stood at
/// pass start, because that is the set `evaluate_selective` seeds
/// `always_report` from. A cell whose branch flips *into* `RAND()` records the
/// input only while it evaluates, so it was never seeded; with `RAND()*0` its
/// value does not move either, and the delta rightly leaves it out. Asserting
/// against the post-pass set panicked on exactly this.
#[cfg(feature = "recalc_verify")]
#[test]
fn verify_liveness_allows_a_cell_that_becomes_volatile_mid_pass() {
    let mut model = new_empty_model().with_recalc_mode(RecalcMode::Verify);
    model._set("D1", "-1");
    model._set("A1", "=IF(D1>0,RAND()*0,0)");
    model.evaluate();
    assert!(!reads_random(&model, (0, 1, 1)));
    model._set("D1", "1");
    model.evaluate();
    assert!(reads_random(&model, (0, 1, 1)));
    assert_eq!(model._get_text("A1"), "0");
    // Steady state: from here A1 is in the pre-pass set on every pass, so the
    // assertion binds and each pass has to report it.
    for value in ["2", "3"] {
        model._set("D1", value);
        model.evaluate();
        assert!(reads_random(&model, (0, 1, 1)));
    }
}

/// The reverse transition keeps the assertion strong. `A1` is volatile entering
/// the pass, so it is in the pre-pass set, seeds `always_report`, and has to be
/// in the delta -- even though `RAND()*0` means its value never moved and the
/// post-pass set no longer contains it. Asserting against the post-pass set
/// would let a pass drop a volatile cell from its delta unnoticed.
#[cfg(feature = "recalc_verify")]
#[test]
fn verify_liveness_still_binds_when_a_cell_leaves_volatility() {
    let mut model = new_empty_model().with_recalc_mode(RecalcMode::Verify);
    model._set("D1", "1");
    model._set("A1", "=IF(D1>0,RAND()*0,0)");
    model.evaluate();
    assert!(reads_random(&model, (0, 1, 1)));
    let _ = model.take_changed_cells();
    model._set("D1", "-1");
    model.evaluate();
    assert!(!reads_random(&model, (0, 1, 1)));
    assert_eq!(model._get_text("A1"), "0");
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => {}
        ChangedSinceRead::Cells(cells) => assert!(
            cells
                .iter()
                .any(|c| (c.sheet, c.row, c.column) == (0, 1, 1)),
            "the pass that dropped A1's volatility did not report it"
        ),
    }
}

// --- Relocated from base/tests/ ------------------------------------------
// These pin engine invariants with plain public-API models and no fuzz
// harness, so they belong in the lib suite: it is the suite the Verify job and
// the nightly mutation run execute, and a `base/tests/*.rs` file is a separate
// compile-and-link that neither of them ever sees.

/// A link write is journaled like a value write, so the incremental delta names
/// the cell. Killed by dropping the `Write::Link` arm of the journal drain.
#[test]
fn cell_link_write_is_reported_in_the_delta() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "hello");
    model.evaluate();
    let _ = model.take_changed_cells();

    model
        .set_cell_link(
            0,
            1,
            1,
            crate::types::Link::External {
                target: "https://ironcalc.com".to_string(),
                tooltip: None,
            },
        )
        .unwrap();
    model.evaluate();
    assert!(delta_names(&mut model, (1, 1)), "set_cell_link");

    model.delete_cell_link(0, 1, 1).unwrap();
    model.evaluate();
    assert!(delta_names(&mut model, (1, 1)), "delete_cell_link");
}

/// A link can sit at a position with no cell -- a structural edit strands them
/// there. Clearing a range that covers it removes the link, and that removal
/// must reach the delta the same way (fuzz seed 18 at 200 steps). Killed by the
/// same mutant, through the other caller.
#[test]
fn range_clear_reports_a_stranded_link_removal() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model
        .set_cell_link(
            0,
            2,
            2,
            crate::types::Link::External {
                target: "https://ironcalc.com".to_string(),
                tooltip: None,
            },
        )
        .unwrap();
    model.evaluate();
    let _ = model.take_changed_cells();
    assert_eq!(model.get_links_list(0).unwrap().len(), 1);

    model
        .range_clear_contents(&crate::expressions::types::Area {
            sheet: 0,
            row: 1,
            column: 1,
            width: 3,
            height: 3,
        })
        .unwrap();
    model.evaluate();
    assert_eq!(
        model.get_links_list(0).unwrap().len(),
        0,
        "link not removed"
    );
    assert!(delta_names(&mut model, (2, 2)), "stranded link removal");
}

/// Whether the incremental delta names `(row, column)` on sheet 0. `Everything`
/// counts: it is the conservative answer, not a miss.
fn delta_names(model: &mut crate::Model, (row, column): (i32, i32)) -> bool {
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => true,
        ChangedSinceRead::Cells(cells) => cells
            .iter()
            .any(|c| (c.sheet, c.row, c.column) == (0, row, column)),
    }
}

/// A dynamic anchor whose last result was 1x1 has no spill cells and is not in
/// the array index, so it behaves as a scalar and must stay on the incremental
/// path. `=LET`, a called `LAMBDA`, `=INDEX` and `=IF` are everyday formulas:
/// falling back to a full pass for them removes the feature silently, with no
/// wrong value to notice.
#[test]
fn scalar_result_dynamic_anchors_stay_incremental() {
    for (formula, expected) in [
        ("=A1+1", "3"),
        ("=LET(y,A1*2,y+1)", "5"),
        ("=LAMBDA(x,x+A1)(3)", "5"),
        ("=INDEX(A1:A3,1)", "2"),
        ("=IF(A1>1,A2,A3)", "1"),
    ] {
        let mut model = new_empty_model().with_recalc_mode(RecalcMode::Incremental);
        for cell in ["A1", "A2", "A3"] {
            model._set(cell, "1");
        }
        model._set("B1", formula);
        model.evaluate();
        let _ = model.take_changed_cells();

        model._set("A1", "2");
        model.evaluate();
        assert_eq!(
            model._get_text("B1"),
            expected,
            "{formula} value after edit"
        );
        assert!(
            matches!(model.take_changed_cells(), ChangedSinceRead::Cells(_)),
            "{formula} fell back to a full pass"
        );
    }
}

/// I1.8 — a reference-returning function records its resolved target at the
/// extent it resolved to, not the extent its reader's walk visited.
///
/// This is I1.3's clipping rule at a *computed* extent. `SUM` clips a
/// whole-column reference to the used range, so the per-cell reads stop at the
/// last populated row; a later write below that row is connected to the
/// formula by the recorded rectangle and by nothing else. The two calls are
/// separate sites, so there is one witness each.
///
/// Kills deleting `trace_rect` from `INDIRECT`'s range branch. Without it the
/// formula's only edges are the clipped per-cell reads, and the write to A500
/// never reaches it.
#[test]
fn indirect_records_its_resolved_extent_not_the_walk() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("B1", "=SUM(INDIRECT(\"A:A\"))");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "1");

    // Below the used range the walk stopped at, so only the rect connects it.
    model._set("A500", "5");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "6");
}

/// I1.8, the `OFFSET` site. Kills deleting `trace_rect` from `fn_offset`.
///
/// The height is spelled out rather than written `A:A` because `OFFSET` over a
/// whole-column argument resolves through a different path; this is the one
/// that reaches `fn_offset`'s own rect.
#[test]
fn offset_records_its_resolved_extent_not_the_walk() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("B1", "=SUM(OFFSET($A$1,0,0,1048576,1))");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "1");

    model._set("A500", "5");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "6");
}
