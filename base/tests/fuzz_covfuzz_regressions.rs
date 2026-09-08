//! Failing shapes found by differential fuzzing: coverage-guided (libFuzzer
//! over a byte->Op decoder; branch recalc-r3-covfuzz) and seeded. Each test
//! replays one minimized artifact through the shared lockstep harness;
//! `assert_clean` pins Full/Incremental/Verify agreement plus the delta
//! contract. They live here rather than in the lib suite because that harness
//! does.
//!
//! One shape per kill class: minimized artifacts arrive in families, and two
//! that die to the same mutant and assert the same observable are one test (see
//! `base/src/recalc/README.md`, "Test discipline"). Each doc comment below names
//! the mutant that kills its shape. Before deleting or merging one of these,
//! re-apply those mutants and check that nothing which died still survives.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;

use common::*;

/// SUBTOTAL over a blocked anchor's would-be footprint: after an unrelated
/// value edit, incremental serves `#REF!` where the full pass has `#SPILL!`.
/// The blocked-anchor class: killed by dropping a blocked (`#SPILL!`) anchor
/// from the array index, and by never rebuilding the blocked-reader set.
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

/// The minimal cycle-miss shape: a COUNT whose range covers itself plus an
/// OFFSET reading it, with no arrays and no structural ops. Overwriting an
/// unrelated formula cell with a number makes incremental serve 0.0 where a
/// full pass keeps `#CIRC!`. The cycle class: killed by disabling never-served
/// seeding, and by dropping the whole-graph rebuild of the set that each full
/// pass does -- the only test in the suite that dies to either.
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

/// Seeded fuzz (seed 251), minimized. A CSE anchor's member is read by a
/// formula the anchor's own range covers, so the cycle closes through the
/// array footprint. The anchor writes its members as evaluation writes and the
/// reader sees an empty cell, so without the footprint-to-anchor read edge
/// nothing tells the scheduler the reader's stored value is an artifact of
/// where the cycle was entered. The only test in the suite that dies when that
/// edge is removed from `evaluate_cell`.
#[test]
fn cse_footprint_cycle_stored_value_diverges_from_a_live_reeval() {
    assert_clean(&[
        Op::ArrayFormula {
            sheet: 0,
            row: 1,
            col: 5,
            width: 1,
            height: 3,
            formula: "=A1:A3+1".to_string(),
        },
        Op::DeleteCols {
            sheet: 0,
            col: 3,
            count: 1,
        },
        Op::Set {
            sheet: 0,
            row: 20,
            col: 5,
            value: "48".to_string(),
        },
        Op::DeleteCols {
            sheet: 0,
            col: 2,
            count: 1,
        },
        Op::Set {
            sheet: 0,
            row: 3,
            col: 1,
            value: "=SUM(B2:C2)".to_string(),
        },
        Op::MoveCols {
            sheet: 0,
            col: 1,
            count: 1,
            delta: 1,
        },
        Op::Evaluate,
        Op::Evaluate,
        Op::DeleteRows {
            sheet: 0,
            row: 16,
            count: 2,
        },
        Op::Evaluate,
    ]);
}

/// Coverage-guided fuzz, run against the never-served rule and minimized. A
/// second cycle at `G19`/`H19` keeps every pass's cone unorderable, so the
/// pass that closes a *new* cycle -- `A1`'s self-range `SUBTOTAL` and the
/// `SEQUENCE` anchor at `D4` that reads `A1` -- is walked by position rather
/// than redone as full. Position order then has to be full's, and full's is
/// two phases: `D4` is an anchor, so full enters the new cycle there and `A1`
/// absorbs the `#CIRC!`; a one-phase row-major walk enters at `A1` and leaves
/// `#CIRC!` on the anchor, which never spills again. `D4` is not a seed on
/// that pass, so no anchor fallback stands in for the phase. The only test in
/// the suite that dies when an evaluation write into an array footprint stops
/// forcing the pass full (`wrote_array_cells`).
#[test]
fn new_cycle_around_an_anchor_places_circ_like_full_phase_one() {
    assert_clean(&[
        Op::Set {
            sheet: 0,
            row: 19,
            col: 7,
            value: "=H19+1".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 19,
            col: 8,
            value: "=G19+1".to_string(),
        },
        Op::Set {
            sheet: 0,
            row: 4,
            col: 4,
            value: "=SEQUENCE(MOD(A1,3)+1)".to_string(),
        },
        Op::Evaluate,
        Op::Set {
            sheet: 0,
            row: 1,
            col: 1,
            value: "=SUBTOTAL(103,A1:D12)".to_string(),
        },
        Op::Evaluate,
    ]);
}
