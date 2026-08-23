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
        // though the delete dropped a member cell.
        x.set_user_input(0, 5, 7, "=IFERROR(G6,-1)".to_string())
            .unwrap();
        v(&x, "Sheet1!G5")
    };
    let (f, i) = (run(RecalcMode::Full), run(RecalcMode::Incremental));
    println!("B3 G5: full={f} incr={i}");
    assert_eq!(f, i);
}
// B5: which formula shapes force a Full pass every time?
#[test]
fn b5_full_fallback_shapes() {
    for f in [
        "=A1+1",
        "=LET(y,A1*2,y+1)",
        "=LAMBDA(x,x+A1)(3)",
        "=INDEX(A1:A3,1)",
        "=IF(A1>1,A2,A3)",
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
        let d = match x.take_changed_cells() {
            ChangedSinceRead::Everything => "FULL".to_string(),
            ChangedSinceRead::Cells(c) => format!("incremental ({} cells)", c.len()),
        };
        println!("B5 {f:<22} -> {d}");
    }
}
