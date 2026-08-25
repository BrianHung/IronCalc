//! Failing shapes found by coverage-guided differential fuzzing (libFuzzer over
//! a byte->Op decoder; branch recalc-r3-covfuzz). Each test replays one
//! minimized crash artifact through the shared lockstep harness; `assert_clean`
//! pins Full/Incremental/Verify agreement plus the delta contract.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;

use common::*;

/// A reader of a dynamic-array anchor (`=IFERROR(E15,-1)`) added AFTER the
/// anchor's spill got blocked keeps the pre-block spilled value (1.0) in the
/// incremental pass while a full pass sees the stored `#SPILL!` (-1).
#[test]
fn covfuzz_blocked_spill_reader_added_after_block_sees_stale_value() {
    assert_clean(&[
        Op::NewName {
            name: "NRANGE".to_string(),
            scope: None,
            formula: "Sheet1!$A$1:$A$8".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 15,
            col: 5,
            value: "=SEQUENCE(3)".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 8,
            col: 7,
            value: "=NRANGE".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 17,
            col: 5,
            value: "=B9+A12".to_string(),
        },
        Op::Evaluate,
        Op::Set {
            sheet: 0,
            row: 7,
            col: 1,
            value: "=IFERROR(E15,-1)".to_string(),
        },
        Op::Evaluate,
        Op::Set {
            sheet: 0,
            row: 7,
            col: 2,
            value: "=PRODUCT(B1:B3)".to_string(),
        },
        Op::Evaluate,
    ]);
}

/// SUBTOTAL over a blocked anchor's would-be footprint: after an unrelated
/// value edit, incremental serves `#REF!` where the full pass has `#SPILL!`.
#[test]
fn covfuzz_blocked_spill_subtotal_reader_diverges_after_unrelated_edit() {
    assert_clean(&[
        Op::Set {
            sheet: 0,
            row: 15,
            col: 5,
            value: "=SEQUENCE(3)".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 1,
            col: 6,
            value: "=LET(x,NCELL,y,D1,x*y)".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 17,
            col: 5,
            value: "=SUM(B3:C9)".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 1,
            col: 4,
            value: "=SUBTOTAL(9,E15:E17)".to_string(),
        },
        Op::Evaluate,
        Op::Set {
            sheet: 0,
            row: 14,
            col: 4,
            value: "#N/A".to_string(),
        },
        Op::Evaluate,
    ]);
}

/// A HYPERLINK whose target reads a spill-displaced cell keeps its stale Text
/// value (and link) in the incremental pass while the full pass stores
/// `#SPILL!` after row moves and a column delete rearrange the anchor.
#[test]
fn covfuzz_moved_blocked_spill_hyperlink_reader_keeps_stale_text() {
    assert_clean(&[
        Op::Set {
            sheet: 0,
            row: 15,
            col: 5,
            value: "=SEQUENCE(3)".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 7,
            col: 7,
            value: "=IF(D1>D2,OFFSET(B1,2,0),C3)".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 17,
            col: 5,
            value: "=SUM(A1:A3*B1:B3)".to_string(),
        },
        Op::MoveRows {
            sheet: 0,
            row: 16,
            count: 2,
            delta: -3,
        },
        Op::MoveRows {
            sheet: 0,
            row: 16,
            count: 2,
            delta: -3,
        },
        Op::MoveRows {
            sheet: 0,
            row: 16,
            count: 2,
            delta: -3,
        },
        Op::MoveRows {
            sheet: 0,
            row: 16,
            count: 2,
            delta: -3,
        },
        Op::Set {
            sheet: 0,
            row: 3,
            col: 3,
            value: "=HYPERLINK(\"https://x.com/\"&E13)".to_string(),
        },
        Op::DeleteCols {
            sheet: 0,
            col: 2,
            count: 1,
        },
        Op::Evaluate,
        Op::Set {
            sheet: 0,
            row: 1,
            col: 4,
            value: "=SUBTOTAL(9,E15:E17)".to_string(),
        },
        Op::Evaluate,
    ]);
}

/// Row/column moves over a CSE array plus a FILTER anchor: incremental places
/// `#CIRC!` on a different cell than the full pass (full marks (0,10,1) too).
#[test]
fn covfuzz_moved_cse_and_filter_place_circ_differently() {
    assert_clean(&[
        Op::Set {
            sheet: 0,
            row: 2,
            col: 6,
            value: "=INDEX(A1:A12,8)".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 8,
            col: 7,
            value: "=FILTER(A1:A6,B1:B6>10)".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 5,
            col: 1,
            value: "=SUMIFS(B1:B10,A1:A3,\">9\")".to_string(),
        },
        Op::MoveRows {
            sheet: 0,
            row: 3,
            count: 2,
            delta: 1,
        },
        Op::ArrayFormula {
            sheet: 0,
            row: 9,
            col: 1,
            width: 1,
            height: 3,
            formula: "=A1:A3+1".to_string(),
        },
        Op::Evaluate,
        Op::MoveRows {
            sheet: 0,
            row: 1,
            count: 1,
            delta: 1,
        },
        Op::MoveCols {
            sheet: 0,
            col: 6,
            count: 1,
            delta: 2,
        },
        Op::Evaluate,
        Op::SetNumber {
            sheet: 0,
            row: 1,
            col: 8,
            value: 84.0,
        },
        Op::Evaluate,
    ]);
}

/// Two `=E15#` readers of a SEQUENCE anchor displaced by InsertRows, plus a
/// ClearContents on a cycle member: incremental's recompute disagrees with a
/// full recompute (#CIRC! membership and anchor-reference values).
#[test]
fn covfuzz_anchor_ref_readers_after_insert_and_clear_diverge() {
    assert_clean(&[
        Op::NewName {
            name: "SCELL".to_string(),
            scope: Some(0),
            formula: "Sheet1!$B$2".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 4,
            col: 6,
            value: "=SEQUENCE(3)+H12".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 6,
            col: 7,
            value: "=SCELL+1".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 12,
            col: 8,
            value: "=A1:A3+B1:B3".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 15,
            col: 7,
            value: "=E15#".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 2,
            col: 1,
            value: "=SEQUENCE(3)+F4".to_string(),
        },
        Op::InsertRows {
            sheet: 0,
            row: 5,
            count: 1,
        },
        Op::Set {
            sheet: 0,
            row: 16,
            col: 8,
            value: "=E15#".to_string(),
        },
        Op::Evaluate,
        Op::Evaluate,
        Op::ClearContents {
            area: (0, 7, 7, 1, 1),
        },
        Op::Evaluate,
    ]);
}

/// Overwriting a variable-height SEQUENCE anchor with a plain formula
/// (`=ISERROR(E8)`) leaves incremental state that a full recompute disagrees
/// with -- the smallest divergence shape found (6 ops).
#[test]
fn covfuzz_overwritten_variable_sequence_anchor_diverges() {
    assert_clean(&[
        Op::Set {
            sheet: 0,
            row: 12,
            col: 5,
            value: "=SEQUENCE(MOD(A5,3)+1)".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 5,
            col: 1,
            value: "=OFFSET(A1,MOD(B4,5),MOD(C4,3))".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 4,
            col: 3,
            value: "=COUNT(A1:D12)".to_string(),
        },
        Op::Evaluate,
        Op::Set {
            sheet: 0,
            row: 12,
            col: 5,
            value: "=ISERROR(E8)".to_string(),
        },
        Op::Evaluate,
    ]);
}

/// The minimal cycle-miss shape: a COUNT whose range covers itself plus an
/// OFFSET reading it, with no arrays and no structural ops. Overwriting an
/// unrelated formula cell with a number makes incremental serve 0.0 where a
/// full pass keeps `#CIRC!`.
#[test]
fn covfuzz_count_offset_cycle_lost_after_number_overwrite() {
    assert_clean(&[
        Op::Set {
            sheet: 0,
            row: 1,
            col: 5,
            value: "=SUM(A1:A5)".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 5,
            col: 1,
            value: "=OFFSET(A1,MOD(B4,5),MOD(C4,3))".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 4,
            col: 3,
            value: "=COUNT(A1:D12)".to_string(),
        },
        Op::Evaluate,
        Op::SetNumber {
            sheet: 0,
            row: 1,
            col: 5,
            value: 84.0,
        },
        Op::Evaluate,
    ]);
}

/// The reverse direction: a NEW `=E20` reader added next to an existing cycle
/// (after row moves/inserts and a SUBTOTAL(103) visibility read) gets a
/// spurious `#CIRC!` from incremental where the full pass computes 1.0.
#[test]
fn covfuzz_new_reader_near_cycle_gets_spurious_circ() {
    assert_clean(&[
        Op::AddSheet {
            name: "Data".to_string(),
        },
        Op::Set {
            sheet: 1,
            row: 4,
            col: 1,
            value: "6".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 19,
            col: 5,
            value: "=SUMIFS(C1:C8,Data!A1:A8,\">4\")".to_string(),
        },
        Op::MoveRows {
            sheet: 0,
            row: 2,
            count: 2,
            delta: -1,
        },
        Op::InsertRows {
            sheet: 0,
            row: 5,
            count: 1,
        },
        Op::Set {
            sheet: 0,
            row: 7,
            col: 4,
            value: "=E20".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 6,
            col: 3,
            value: "=SUBTOTAL(103,A1:D12)".to_string(),
        },
        Op::Evaluate,
        Op::Set {
            sheet: 0,
            row: 4,
            col: 7,
            value: "=E20".to_string(),
        },
        Op::Evaluate,
    ]);
}
