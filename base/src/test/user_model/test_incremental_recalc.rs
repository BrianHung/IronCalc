#![allow(clippy::unwrap_used)]
#![allow(clippy::panic)]

use crate::recalc::Input;
use crate::test::util::{incremental_mode, new_empty_model};
use crate::types::CellType;
use crate::{ChangedSinceRead, UserModel};

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
fn incremental_matches_full_on_a_chain() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("B1", "=A1*2");
    model._set("C1", "=B1+1");
    model.evaluate();

    model._set("A1", "10");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "20");
    assert_eq!(model._get_text("C1"), "21");
}

#[test]
fn incremental_error_is_not_a_same_text_literal() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("B1", "=1/A1");
    model.evaluate();
    assert_eq!(model._get_cell("B1").get_type(), CellType::Number);

    // Value edit, not a formula write: `=…` would force_full and skip Verify.
    // CellValue stores errors as text, so type is the only Error vs "#DIV/0!" distinction.
    model._set("A1", "0");
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_cell("B1").get_type(), CellType::ErrorValue);
    assert_eq!(model._get_text("B1"), "#DIV/0!");
}

#[test]
fn volatile_hidden_in_defined_name_lambda_is_detected() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    // A lambda stored as a defined name, hiding a volatile function. Passed bare
    // to MAP it appears as a defined-name node, not an inline lambda.
    model
        .new_defined_name("V", None, "=LAMBDA(x, x + RAND())")
        .unwrap();
    model._set("A1", "1");
    model._set("B1", "=MAP(A1, V)");
    model.evaluate();

    // B1 (sheet 0, row 1, column 2) must be volatile so the incremental path
    // re-rolls it every pass, matching a full recompute.
    assert!(reads_random(&model, (0, 1, 2)));
}

#[test]
fn incremental_coexists_with_arrays() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("B1", "1");
    model._set("B2", "2");
    model._set("A1", "=B1:B2"); // spills A1:A2
    model._set("D1", "10"); // independent
    model._set("D2", "=D1+1");
    model.evaluate();

    model._set("D1", "20"); // unrelated to the spill
    model.evaluate();
    assert_eq!(model._get_text("D2"), "21");
    assert_eq!(model._get_text("A2"), "2"); // spill untouched

    model._set("B2", "9"); // feeds the spill: falls back to full
    model.evaluate();
    assert_eq!(model._get_text("A2"), "9"); // re-spilled
}

#[test]
fn incremental_handles_row_insert() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    for row in 1..=5 {
        model._set(&format!("A{row}"), &row.to_string());
    }
    model._set("C1", "=SUM(A1:A5)");
    model._set("E1", "=ROWS(A1:A5)");
    model.evaluate();

    // An insert only shifts references, so the graph shifts its edges rather
    // than rebuilding: the next pass stays incremental.
    model.insert_rows(0, 3, 2).unwrap(); // A1:A5 -> A1:A7
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("C1"), "15"); // SUM unchanged
    assert_eq!(model._get_text("E1"), "7"); // ROWS grew
}

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
fn incremental_column_insert_stays_incremental() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("B1", "2");
    model._set("D1", "=A1+B1");
    model.evaluate();

    model.insert_columns(0, 2, 1).unwrap(); // B shifts to C, D to E
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("E1"), "3");
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
fn incremental_structural_edit_below_volatile_stays_incremental() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "=RAND()");
    model._set("C10", "=1+1");
    model.evaluate();

    // Inserting below the volatile leaves it in place, so the edit can stay
    // incremental.
    model.insert_rows(0, 5, 1).unwrap();
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("C11"), "2");
}

#[test]
fn incremental_shares_one_range_vertex() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    for row in 1..=10 {
        model._set(&format!("A{row}"), &row.to_string());
    }
    for row in 1..=50 {
        model._set(&format!("C{row}"), "=SUM(A1:A10)");
    }
    model.evaluate();

    model._set("A5", "100"); // sum 55 -> 150
    model.evaluate();
    for row in 1..=50 {
        assert_eq!(model._get_text(&format!("C{row}")), "150");
    }
}

#[test]
fn incremental_recomputes_volatiles() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("C1", "5");
    model._set("A1", "=INDIRECT(\"C1\")"); // reads C1 dynamically
    model.evaluate();
    assert_eq!(model._get_text("A1"), "5");

    model._set("C1", "99"); // A1's hidden target; without volatiles A1 stays 5
    model.evaluate();
    assert_eq!(model._get_text("A1"), "99");
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
fn incremental_tracks_defined_name_references() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    let sheet = model.workbook.worksheets[0].get_name();
    model
        .new_defined_name("MyRef", None, &format!("{sheet}!$B$1"))
        .unwrap();
    model._set("B1", "5");
    model._set("A1", "=MyRef");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "5");

    model._set("B1", "99"); // edit the name's target
    model.evaluate();
    assert_eq!(model._get_text("A1"), "99");
}

#[test]
fn incremental_nested_dynamic_range_tracks_interior() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("A5", "0");
    model._set("A10", "1");
    // Parentheses keep the colon as OpRangeKind under SUM. Static edges are
    // only the endpoints; A5 is missed unless the SUM is marked volatile.
    model._set("B1", "=SUM((A1):(A10))");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "2");

    model._set("A5", "10");
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("B1"), "12");
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
fn incremental_range_clear_contents_forces_full() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("B1", "=A1");
    model._set("C1", "=B1");
    model._set("D1", "0");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "1");

    model._set("D1", "5"); // pending edit arms incremental
    model
        .range_clear_contents(&crate::expressions::types::Area {
            sheet: 0,
            row: 1,
            column: 2,
            width: 1,
            height: 1,
        })
        .unwrap(); // clears B1; must fall back to full
    model.evaluate();
    assert_eq!(model._get_text("C1"), ""); // B1 cleared, not the stale 1
}

#[test]
fn incremental_set_array_formula_forces_full() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("B1", "1");
    model._set("B2", "2");
    model._set("D1", "10");
    model._set("D2", "=D1+1");
    model.evaluate();

    model._set("D1", "20"); // pending edit arms incremental
    model
        .set_user_array_formula(0, 1, 1, 1, 2, "=B1:B2")
        .unwrap(); // spills A1:A2
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1");
    assert_eq!(model._get_text("A2"), "2"); // spill produced, not #ERROR!
}

#[test]
fn incremental_offset_reads_dynamic_target() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("C1", "10");
    model._set("D2", "=C1");
    model._set("A1", "=OFFSET(D1,1,0)"); // reads D2, a target no static edge captures
    model._set("B1", "=A1"); // static dependent of the dynamic ref
    model.evaluate();
    assert_eq!(model._get_text("A1"), "10");
    assert_eq!(model._get_text("B1"), "10");

    model._set("C1", "20"); // D2 -> 20; A1 must follow, not read a stale D2
    model.evaluate();
    assert_eq!(model._get_text("A1"), "20");
    assert_eq!(model._get_text("B1"), "20");
}

#[test]
fn incremental_chained_offset_reads_updated_target() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("C1", "10");
    model._set("E2", "=C1");
    model._set("D2", "=OFFSET(E1,1,0)"); // dynamic ref whose target is E2
    model._set("A1", "=OFFSET(D1,1,0)"); // dynamic ref whose target is D2
    model.evaluate();
    assert_eq!(model._get_text("A1"), "10");

    // No static edge A1→D2. If D2 is still Evaluated, A1 reads 10.
    model._set("C1", "20");
    model.evaluate();
    assert_eq!(model._get_text("D2"), "20");
    assert_eq!(model._get_text("A1"), "20");
}

#[test]
fn incremental_offset_through_helper_reads_updated_target() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("C1", "10");
    model._set("F2", "=C1");
    model._set("E2", "=OFFSET(F1,1,0)"); // reads F2
    model._set("D2", "=E2"); // static helper — in the cone, not a seed
    model._set("A1", "=OFFSET(D1,1,0)"); // reads D2; no edge D2→A1
    model.evaluate();
    assert_eq!(model._get_text("A1"), "10");
    let _ = model.take_changed_cells();

    model._set("C1", "20");
    model.evaluate();
    assert_eq!(model._get_text("E2"), "20");
    assert_eq!(model._get_text("D2"), "20");
    assert_eq!(model._get_text("A1"), "20");
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("expected incremental delta");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(2, 4))); // D2 helper
    assert!(changed.contains(&(1, 1))); // A1 OFFSET
}

#[test]
fn incremental_indirect_through_helper_reads_updated_target() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("C1", "10");
    model._set("F2", "=C1");
    model._set("E2", "=INDIRECT(\"F2\")");
    model._set("D2", "=E2");
    model._set("A1", "=INDIRECT(\"D2\")");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "10");
    let _ = model.take_changed_cells();

    model._set("C1", "20");
    model.evaluate();
    assert_eq!(model._get_text("E2"), "20");
    assert_eq!(model._get_text("D2"), "20");
    assert_eq!(model._get_text("A1"), "20");
    // INDIRECT is stored as a 1×1 dynamic array, so the cone takes the
    // array/spill full path. That is Everything, not an incremental delta.
    assert_eq!(model.take_changed_cells(), ChangedSinceRead::Everything);
}

#[test]
fn incremental_offset_does_not_force_full_on_unrelated_edit() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("C1", "10");
    model._set("A1", "=OFFSET(C1,0,0)");
    model._set("Z1", "1");
    model.evaluate();
    assert_eq!(model.take_changed_cells(), ChangedSinceRead::Everything);

    // OFFSET's actual target is a traced edge, not a role set. An unrelated
    // edit must stay incremental instead of falling back to a full workbook pass.
    model._set("Z1", "2");
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("OFFSET must not force a workbook-wide full pass");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(1, 26))); // Z1
    assert!(!changed.contains(&(1, 1))); // A1 OFFSET ran; value did not move
}

#[test]
fn incremental_hyperlink_over_offset_keeps_link_on_unrelated_edit() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("C1", "http://a.com");
    model._set("A1", "=OFFSET(C1,0,0)");
    model._set("B1", "=HYPERLINK(A1,\"click\")");
    model._set("Z1", "1");
    model.evaluate();
    let dynamic_at = |model: &crate::Model| {
        model
            .get_links_list(0)
            .unwrap()
            .into_iter()
            .filter(|l| l.dynamic)
            .map(|l| (l.row, l.column))
            .collect::<Vec<_>>()
    };
    assert_eq!(dynamic_at(&model), vec![(1, 2)]); // B1

    model._set("Z1", "2"); // A1 unchanged; B1 must not lose its URL
    model.evaluate();
    assert_eq!(model._get_text("B1"), "click");
    assert_eq!(dynamic_at(&model), vec![(1, 2)]);
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
fn incremental_reports_locale_change() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1234.5");
    model._set("B1", "=TEXT(A1,\"#,##0.00\")");
    model._set("Z1", "0"); // independent
    model.evaluate();
    let _ = model.take_changed_cells();

    model._set("Z1", "1"); // a pending edit that alone would be a small delta
    model.set_locale("de").unwrap();
    // A locale change re-renders every formatted value, so no small delta
    // describes it; the record reports everything changed.
    assert_eq!(model.take_changed_cells(), ChangedSinceRead::Everything);
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

#[test]
fn incremental_tracks_function_read_references() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("A2", "2");
    model._set("A3", "3");
    model._set("B1", "=SUBTOTAL(9,A1:A3)");
    model._set("D1", "1"); // selector
    model._set("E1", "10");
    model._set("F1", "20");
    model._set("D2", "=CHOOSE(D1,E1,F1)");
    model.evaluate();

    model._set("A1", "10"); // inside SUBTOTAL's range
    model._set("E1", "99"); // CHOOSE's selected branch
    model.evaluate();
    assert_eq!(model._get_text("B1"), "15"); // 1+2+3 -> 10+2+3
    assert_eq!(model._get_text("D2"), "99"); // CHOOSE -> E1
}

#[test]
fn incremental_tracks_named_lambda_body_references() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    let sheet = model.workbook.worksheets[0].get_name();
    model
        .new_defined_name("TAX", None, &format!("=LAMBDA(amt, amt + {sheet}!$B$1)"))
        .unwrap();
    model._set("B1", "5"); // closed over
    model._set("C1", "100"); // argument
    model._set("A1", "=TAX(C1)");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "105");

    model._set("B1", "9"); // edit the closed-over cell
    model.evaluate();
    assert_eq!(model._get_text("A1"), "109");
}

#[test]
fn incremental_tracks_volatile_in_lambda_body() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model
        .new_defined_name("VOL", None, "=LAMBDA(a, a + INDIRECT(\"A1\"))")
        .unwrap();
    model._set("A1", "5"); // read dynamically, no static edge
    model._set("B1", "=VOL(10)");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "15");

    model._set("A1", "50");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "60");
}

#[test]
fn incremental_tracks_dynamic_range_operator() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "10");
    model._set("A2", "20");
    model._set("A3", "30");
    model._set("C1", "3"); // row count
    model._set("B1", "=SUM(A1:OFFSET(A1,C1-1,0))"); // sums A1 down to row C1
    model.evaluate();
    assert_eq!(model._get_text("B1"), "60"); // 10+20+30

    model._set("A2", "25"); // interior cell
    model.evaluate();
    assert_eq!(model._get_text("B1"), "65");
}

#[test]
fn incremental_insert_below_dynamic_array_rebuilds_spill() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("B1", "1");
    model._set("B2", "2");
    model._set("A1", "=B1:B2"); // spills A1:A2
    model._set("Z1", "0");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1");
    assert_eq!(model._get_text("A2"), "2");

    model._set("Z1", "1"); // dirty an unrelated cell
    model.insert_rows(0, 6, 1).unwrap();
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1");
    assert_eq!(model._get_text("A2"), "2"); // spill left intact, not #ERROR!
}

#[test]
fn incremental_insert_below_dynamic_array_and_volatile_stays_incremental() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("B1", "1");
    model._set("B2", "2");
    model._set("A1", "=B1:B2"); // spills A1:A2, entirely above the insert
    model._set("C10", "=1+1");
    model.evaluate();

    model.insert_rows(0, 5, 1).unwrap();
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1");
    assert_eq!(model._get_text("A2"), "2");
    assert_eq!(model._get_text("C11"), "2");
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
    assert_eq!(inc._get_cell("E17").get_type(), full._get_cell("E17").get_type());
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
fn take_changed_cells_reports_structural_edit_delta() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "10");
    model._set("B10", "=A1+1"); // depends on A1, value 11
    model.evaluate();
    let _ = model.take_changed_cells(); // clear the mode-switch full

    // Delete row 1: A1 is removed so B10's reference dangles, and B10 shifts up
    // to B9. Data cells in the shift band are not in the dirty cone, so the
    // delta cannot name every moved cell. Report Everything.
    model.delete_rows(0, 1, 1).unwrap();
    model.evaluate();

    assert_eq!(model.take_changed_cells(), ChangedSinceRead::Everything);
    assert_eq!(model._get_text("B9"), "#REF!");
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
fn take_changed_cells_survives_redundant_evaluate_with_offset() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("C1", "10");
    model._set("A1", "=OFFSET(C1,0,0)");
    model._set("B1", "=C1+1");
    model.evaluate();
    let _ = model.take_changed_cells();

    model._set("C1", "20");
    model.evaluate();
    model.evaluate(); // OFFSET does not re-roll; must keep the incremental delta
    let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
        panic!("OFFSET must not wipe the delta to Everything");
    };
    let changed: std::collections::HashSet<(i32, i32)> =
        cells.iter().map(|c| (c.row, c.column)).collect();
    assert!(changed.contains(&(1, 3))); // C1
    assert!(changed.contains(&(1, 2))); // B1
}

#[test]
fn phase_two_restores_memo_for_skipped_cone_cells() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("E2", "5");
    model._set("D2", "=E2");
    model._set("A1", "=OFFSET(D2,0,0)");
    model.evaluate();
    model._set("Z1", "1");
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert!(
        model.cells.contains_key(&(0, 2, 4)),
        "D2 must stay Evaluated after phase 2 skips it"
    );
    assert_eq!(model._get_text("A1"), "5");
    assert_eq!(model._get_text("D2"), "5");
}

#[test]
fn redundant_evaluate_with_volatile_reports_full_change() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model._set("B1", "=A1+1");
    model._set("C1", "=RAND()"); // volatile: re-rolls on every full pass
    model.evaluate();
    let _ = model.take_changed_cells(); // clear the mode-switch full

    model._set("A1", "5");
    model.evaluate(); // incremental edit
    let _ = model.take_changed_cells(); // clear that delta

    model.evaluate(); // redundant full: re-rolls C1, so not a no-op
    assert_eq!(model.take_changed_cells(), ChangedSinceRead::Everything); // not Cells([])
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
fn incremental_cell_name_survives_insert() {
    // Verify's interleaved full pass rebuilds the graph and hides this class.
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    let sheet = model.workbook.worksheets[0].get_name();
    model._set("A10", "42");
    model
        .new_defined_name("MyRef", None, &format!("{sheet}!$A$10"))
        .unwrap();
    model._set("B1", "=MyRef");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "42");

    // Insert between the formula and the name target so B1 stays put while
    // the old A10 shifts to A11. The name still reads $A$10.
    model.insert_rows(0, 5, 1).unwrap();
    model.evaluate();
    assert_eq!(model._get_text("B1"), "");

    model._set("A10", "99");
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("B1"), "99");
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
fn incremental_formulatext_sees_displaced_formula() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("B1", "1");
    model._set("A1", "=SUM(B1)");
    model._set("C1", "=FORMULATEXT(A1)");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "=SUM(B1)");

    model.insert_rows(0, 1, 1).unwrap();
    model.evaluate();
    assert_eq!(model._get_text("C2"), "=SUM(B2)");
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
fn incremental_overwrite_rand_clears_volatile() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "=RAND()");
    model.evaluate();
    assert!(reads_random(&model, (0, 1, 1)));

    model._set("A1", "5");
    model.evaluate();
    assert!(!reads_random(&model, (0, 1, 1)));
    assert_eq!(model._get_text("A1"), "5");

    model._set("B1", "1");
    model.evaluate();
    model._set("B1", "2");
    model.evaluate();
    assert_eq!(model._get_text("A1"), "5");
    assert!(!reads_random(&model, (0, 1, 1)));
}

#[test]
fn incremental_update_cell_apis_clear_volatile_roles() {
    for overwrite in [
        |model: &mut crate::Model| model.update_cell_with_number(0, 1, 1, 5.0).unwrap(),
        |model: &mut crate::Model| model.update_cell_with_text(0, 1, 1, "5").unwrap(),
        |model: &mut crate::Model| model.update_cell_with_bool(0, 1, 1, true).unwrap(),
    ] {
        let mut model = new_empty_model().with_recalc_mode(incremental_mode());
        model._set("A1", "=RAND()");
        model.evaluate();
        assert!(reads_random(&model, (0, 1, 1)));

        overwrite(&mut model);
        model.evaluate();
        assert!(!reads_random(&model, (0, 1, 1)));
        let _ = model.take_changed_cells();

        model._set("B1", "1");
        model.evaluate();
        model._set("B1", "2");
        model.evaluate();
        assert!(!reads_random(&model, (0, 1, 1)));

        let ChangedSinceRead::Cells(cells) = model.take_changed_cells() else {
            panic!("expected incremental delta");
        };
        let changed: std::collections::HashSet<(i32, i32)> =
            cells.iter().map(|c| (c.row, c.column)).collect();
        assert!(
            !changed.contains(&(1, 1)),
            "overwritten RAND must not stay in later deltas"
        );
    }
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
fn incremental_rejected_write_does_not_poison_evaluate() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("A1", "1");
    model.evaluate();
    assert!(model.set_user_input(1, 9, 3, "84".to_string()).is_err());
    model.evaluate();
    assert_eq!(model._get_text("A1"), "1");
}

#[test]
fn incremental_blocked_spill_respills_after_insert() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("E3", "7");
    model._set("E1", "=SEQUENCE(3)");
    model.evaluate();
    assert_eq!(model._get_text("E1"), "#SPILL!");

    model.insert_rows(0, 2, 2).unwrap();
    model.evaluate();
    assert_eq!(model._get_text("E1"), "1");
    assert_eq!(model._get_text("E2"), "2");
    assert_eq!(model._get_text("E3"), "3");
}

#[test]
fn incremental_empty_passthrough_counts_as_blank() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("C1", "=COUNTBLANK(D1:E1)");
    model._set("D1", "=A1");
    model._set("E1", "x");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "1");

    model._set("E1", "y");
    model.evaluate();
    assert_eq!(model._get_text("C1"), "1");
}

#[test]
fn incremental_empty_passthrough_concat() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("B1", "=D1&G1");
    model._set("D1", "=IF(TRUE,A1,1)");
    model._set("G1", "1");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "1");

    model._set("G1", "2");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "2");
}

#[test]
fn incremental_sumifs_index_criteria_tracks_expanded_range() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    for row in 1..=10 {
        model.set_user_input(0, row, 1, "1".to_string()).unwrap();
        model.set_user_input(0, row, 4, "1".to_string()).unwrap();
    }
    model._set("E1", "=SUMIFS(D1:D10,INDEX(A1:C3,0,1),\">0\")");
    model.evaluate();
    assert_eq!(model._get_text("E1"), "10");

    model._set("A7", "0");
    model.evaluate();
    assert_eq!(model._get_text("E1"), "9");
}

#[test]
fn incremental_sumifs_let_criteria_tracks_expanded_range() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    for row in 1..=10 {
        model.set_user_input(0, row, 1, "1".to_string()).unwrap();
        model.set_user_input(0, row, 4, "1".to_string()).unwrap();
    }
    model._set("E1", "=LET(r,A1:A3,SUMIFS(D1:D10,r,\">0\"))");
    model.evaluate();
    assert_eq!(model._get_text("E1"), "10");

    model._set("A7", "0");
    model.evaluate();
    assert_eq!(model._get_text("E1"), "9");
}

#[test]
fn incremental_offset_cone_delta_only_net_movers() {
    for _ in 0..40 {
        let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
        model._set("C1", "0");
        model._set("D1", "10");
        model._set("E1", "=C1*10");
        model._set("A1", "=OFFSET(D1,0,C1)");
        model._set("G1", "=A1+1");
        model.evaluate();
        let _ = model.take_changed_cells();
        model._set("C1", "1");
        model.evaluate();
        assert_eq!(model._get_text("A1"), "10");
        match model.take_changed_cells() {
            ChangedSinceRead::Everything => panic!("expected a cells delta"),
            ChangedSinceRead::Cells(cells) => {
                assert!(
                    !cells
                        .iter()
                        .any(|c| c.row == 1 && (c.column == 1 || c.column == 7)),
                    "unchanged A1/G1 in delta: {cells:?}"
                );
            }
        }
    }
}

#[test]
fn incremental_redundant_evaluate_reports_late_spill() {
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    model._set("E15", "=SEQUENCE(3)");
    model._set("A1", "=E15#");
    model.evaluate();
    let _ = model.take_changed_cells();
    model.evaluate();
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => {}
        ChangedSinceRead::Cells(cells) => {
            if model._get_text("A2") == "2" {
                assert!(
                    cells.iter().any(|c| c.row == 2 && c.column == 1),
                    "A2 spilled on the redundant pass but is missing from the delta: {cells:?}"
                );
            }
        }
    }
}

#[test]
fn incremental_blocked_spill_sum_agrees_with_full() {
    // A blocked dynamic array must not skip the arrays→Full guard, and SUM of
    // the spill column must see #SPILL! rather than a stale number.
    fn run(mode: crate::RecalcMode) -> (String, String) {
        let mut model = new_empty_model().with_recalc_mode(mode);
        model._set("G1", "=SUM(E:E)");
        model._set("E8", "=SUM(B3:C9)");
        model._set("E19", "=A1:A3*2");
        model._set("E20", "=OFFSET(A1,1,1)");
        model.evaluate();
        model._set("C4", "5");
        model.evaluate();
        (model._get_text("E19"), model._get_text("G1"))
    }
    assert_eq!(
        run(crate::RecalcMode::Full),
        run(crate::RecalcMode::Incremental)
    );
}

#[test]
fn stored_empty_formula_is_live_empty() {
    let mut model = new_empty_model().with_recalc_mode(incremental_mode());
    model._set("B1", "=A1");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "");
    match model.workbook.worksheet(0).unwrap().cell(1, 2) {
        Some(crate::types::Cell::CellFormula {
            v: crate::types::FormulaValue::Empty,
            ..
        }) => {}
        other => panic!("expected stored Empty, got {other:?}"),
    }
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
fn sumifs_criteria_from_index_is_tracked() {
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    for row in 1..=10 {
        model.set_user_input(0, row, 1, "1".to_string()).unwrap();
        model.set_user_input(0, row, 4, "1".to_string()).unwrap();
    }
    model._set("E1", "=SUMIFS(D1:D10,INDEX(A1:C3,0,1),\">0\")");
    model.evaluate();
    assert_eq!(model._get_text("E1"), "10");
    let _ = model.take_changed_cells();

    model._set("A7", "0");
    model.evaluate();
    assert_eq!(model._get_text("E1"), "9");
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => panic!("SUMIFS INDEX criteria must stay Incremental"),
        ChangedSinceRead::Cells(_) => {}
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
    assert_eq!(model._get_text("B1"), "");

    model._set("A10", "99");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "99");
}

#[test]
fn name_reader_redirty_on_insert_sheet_scoped() {
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    let sheet = model.workbook.worksheets[0].get_name();
    model._set("A10", "42");
    model
        .new_defined_name("LocalRef", Some(0), &format!("{sheet}!$A$10"))
        .unwrap();
    model._set("B1", "=LocalRef");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "42");

    model.insert_rows(0, 5, 1).unwrap();
    model.evaluate();
    assert_eq!(model._get_text("B1"), "");

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

#[test]
fn formula_edit_stays_incremental() {
    let mut model = new_empty_model().with_recalc_mode(crate::RecalcMode::Incremental);
    model._set("A1", "1");
    model._set("B1", "=A1+1");
    model.evaluate();
    let _ = model.take_changed_cells();

    model._set("B1", "=A1+2");
    flush_writes(&mut model);
    assert!(!model.graph.should_recompute_full());
    model.evaluate();
    assert_eq!(model._get_text("B1"), "3");
    match model.take_changed_cells() {
        ChangedSinceRead::Everything => panic!("formula edit must stay Incremental"),
        ChangedSinceRead::Cells(_) => {}
    }

    model._set("A1", "5");
    model.evaluate();
    assert_eq!(model._get_text("B1"), "7");
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

/// SEQUENCE + a `#` spill-ref can take a second Full pass to materialize
/// spill cells. An unrelated write on another sheet must not leave those
/// cells missing on Incremental (fuzz seed 52).
#[test]
fn sequence_hash_spill_survives_unrelated_other_sheet_edit() {
    let mut inc = new_empty_model().with_recalc_mode(incremental_mode());
    inc.add_sheet("Data").unwrap();
    inc.set_user_input(0, 15, 5, "=SEQUENCE(3)".to_string()).unwrap();
    inc.set_user_input(0, 13, 7, "=E15#".to_string()).unwrap();
    inc.evaluate();
    let g14_after_first = inc._get_text("G14");

    let mut full = new_empty_model();
    full.add_sheet("Data").unwrap();
    full.set_user_input(0, 15, 5, "=SEQUENCE(3)".to_string()).unwrap();
    full.set_user_input(0, 13, 7, "=E15#".to_string()).unwrap();
    full.evaluate();

    inc.set_user_input(1, 4, 1, "38".to_string()).unwrap();
    inc.evaluate();
    full.set_user_input(1, 4, 1, "38".to_string()).unwrap();
    full.evaluate();

    assert_eq!(
        inc._get_text("G14"),
        full._get_text("G14"),
        "after first evaluate Incremental G14={g14_after_first:?}"
    );
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

#[test]
fn undo_redo_under_incremental_stays_incremental() {
    let mut model = UserModel::new_empty("m", "en", "UTC", "en")
        .unwrap()
        .with_recalc_mode(incremental_mode());
    model.set_user_input(0, 1, 1, "1").unwrap();
    model.set_user_input(0, 1, 2, "=A1+1").unwrap();
    model.evaluate();
    let _ = model.model.take_changed_cells();
    model.set_user_input(0, 1, 1, "5").unwrap();
    model.evaluate();
    assert_eq!(model.get_formatted_cell_value(0, 1, 2).unwrap(), "6");
    model.undo().unwrap();
    model.evaluate();
    assert_eq!(model.get_formatted_cell_value(0, 1, 2).unwrap(), "2");
    match model.model.take_changed_cells() {
        ChangedSinceRead::Everything => panic!("undo of a value edit must stay Incremental"),
        ChangedSinceRead::Cells(_) => {}
    }
}
