#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use ironcalc_base::{ChangedSinceRead, Model, RecalcMode};
use std::time::Instant;
fn m(mode: RecalcMode) -> Model<'static> {
    Model::new_empty("t", "en", "UTC", "en")
        .unwrap()
        .with_recalc_mode(mode)
}
fn v(x: &Model, r: &str) -> String {
    format!("{:?}", x.get_cell_value_by_ref(r).unwrap())
}

// B1: per-pass cost must not grow with the number of passes.
#[test]
fn b1_pass_cost_is_flat_over_model_lifetime() {
    let mut x = m(RecalcMode::Incremental);
    for r in 1..=3 {
        x.set_user_input(0, r, 1, "1".to_string()).unwrap();
    }
    x.set_user_input(0, 1, 2, "=SUM(A1:A3)".to_string())
        .unwrap();
    x.evaluate();
    let mut windows = vec![];
    for w in 0..4 {
        let t = Instant::now();
        for i in 0..5000 {
            x.set_user_input(0, 1, 1, format!("{}", (w * 5000 + i) % 97))
                .unwrap();
            x.evaluate();
        }
        windows.push(t.elapsed().as_millis());
    }
    println!("B1 5k-pass windows (ms): {windows:?}");
    assert!(
        windows[3] < windows[0] * 2 + 50,
        "per-pass cost grows with pass count: {windows:?}"
    );
}
// B3: CSE array range cell after delete_columns; new formula reads it.
#[test]
fn b3_cse_range_cell_guard() {
    let run = |mode| {
        let mut x = m(mode);
        x.set_user_array_formula(0, 4, 8, 1, 2, "=A1:A3+1").unwrap();
        x.evaluate();
        x.delete_columns(0, 7, 1).unwrap();
        // A reader over the displaced CSE area must agree across modes even
        // though the delete dropped a member cell. The reader sits outside the
        // rectangle: writing inside it is rejected (review_cse_member_writes).
        x.set_user_input(0, 5, 8, "=IFERROR(G5,-1)".to_string())
            .unwrap();
        v(&x, "Sheet1!H5")
    };
    let (f, i) = (run(RecalcMode::Full), run(RecalcMode::Incremental));
    println!("B3 G5: full={f} incr={i}");
    assert_eq!(f, i);
}
// B5: a dynamic anchor whose last result was 1x1 behaves as a scalar and must
// not force a Full pass: `=LET`, a called LAMBDA, and `=INDEX` are everyday
// formulas, and falling back for them silently removes the feature.
#[test]
fn b5_scalar_dynamic_anchors_stay_incremental() {
    for (f, expected) in [
        ("=A1+1", "Number(3.0)"),
        ("=LET(y,A1*2,y+1)", "Number(5.0)"),
        ("=LAMBDA(x,x+A1)(3)", "Number(5.0)"),
        ("=INDEX(A1:A3,1)", "Number(2.0)"),
        ("=IF(A1>1,A2,A3)", "Number(1.0)"),
    ] {
        let mut x = m(RecalcMode::Incremental);
        for r in 1..=3 {
            x.set_user_input(0, r, 1, "1".to_string()).unwrap();
        }
        x.set_user_input(0, 1, 2, f.to_string()).unwrap();
        x.evaluate();
        let _ = x.take_changed_cells();
        x.set_user_input(0, 1, 1, "2".to_string()).unwrap();
        x.evaluate();
        assert_eq!(v(&x, "Sheet1!B1"), expected, "{f} value after edit");
        let d = match x.take_changed_cells() {
            ChangedSinceRead::Everything => "FULL".to_string(),
            ChangedSinceRead::Cells(c) => format!("incremental ({} cells)", c.len()),
        };
        println!("B5 {f:<22} -> {d}");
        assert!(
            d.starts_with("incremental"),
            "{f} fell back to a Full pass: {d}"
        );
    }
}

// B5b: a 1x1 dynamic anchor whose result grows must spill correctly; the pass
// that creates the spill falls back to Full via the post-pass arrays check.
// Every step compares Incremental against a Full model fed the same edits.
#[test]
fn b5_growing_dynamic_anchor_spills() {
    let mut inc = m(RecalcMode::Incremental);
    let mut full = m(RecalcMode::Full);
    let both = |ops: &[(i32, i32, &str)], inc: &mut Model, full: &mut Model| {
        for &(r, c, val) in ops {
            inc.set_user_input(0, r, c, val.to_string()).unwrap();
            full.set_user_input(0, r, c, val.to_string()).unwrap();
        }
        inc.evaluate();
        full.evaluate();
        for cell in ["Sheet1!F1", "Sheet1!F2", "Sheet1!F3"] {
            assert_eq!(v(inc, cell), v(full, cell), "{cell} diverged");
        }
    };
    both(
        &[
            (1, 1, "1"),
            (1, 2, "10"),
            (2, 2, "20"),
            // Dynamic at parse (a range in one branch); 1x1 while A1=1.
            (1, 6, "=IF(A1=1,5,B1:B2)"),
        ],
        &mut inc,
        &mut full,
    );
    assert_eq!(v(&inc, "Sheet1!F1"), "Number(5.0)");
    // A precedent edit while the result is 1x1 stays incremental.
    let _ = inc.take_changed_cells();
    both(&[(1, 1, "1")], &mut inc, &mut full);
    // Flip: the result becomes B1:B2 and must spill into F1:F2.
    both(&[(1, 1, "2")], &mut inc, &mut full);
    assert_eq!(v(&inc, "Sheet1!F1"), "Number(10.0)");
    assert_eq!(v(&inc, "Sheet1!F2"), "Number(20.0)");
    // And back: the spill retracts.
    both(&[(1, 1, "1")], &mut inc, &mut full);
    assert_eq!(v(&inc, "Sheet1!F1"), "Number(5.0)");
}

// B6: the cost of an incremental pass must depend on the cone, not on the
// total number of cells in the workbook. A whole-workbook walk per pass
// (the old post-pass collect_array_cells) makes this ratio grow with size.
#[test]
fn b6_pass_cost_is_independent_of_workbook_size() {
    let cost = |unrelated: i32| -> u128 {
        let mut x = m(RecalcMode::Incremental);
        // A 200-cell chain in column A: the cone for an A1 edit.
        x.set_user_input(0, 1, 1, "1".to_string()).unwrap();
        for r in 2..=200 {
            x.set_user_input(0, r, 1, format!("=A{}+1", r - 1)).unwrap();
        }
        // Unrelated formulas, far away, never in the cone.
        for i in 0..unrelated {
            x.set_user_input(0, 1000 + i / 20, 10 + i % 20, "=1+0".to_string())
                .unwrap();
        }
        x.evaluate();
        let t = Instant::now();
        for i in 0..300 {
            x.set_user_input(0, 1, 1, format!("{}", i % 97)).unwrap();
            x.evaluate();
        }
        t.elapsed().as_micros()
    };
    let small = cost(2_000).max(1);
    let large = cost(32_000);
    println!("B6 300 passes, 200-cone: 2k cells {small}us, 32k cells {large}us");
    assert!(
        large < small * 3 + 200_000,
        "pass cost grows with workbook size: 2k={small}us 32k={large}us"
    );
}
