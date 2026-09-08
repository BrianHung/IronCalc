#![allow(clippy::unwrap_used)]
#![allow(clippy::print_stdout)]
use crate::{Model, RecalcMode};
use std::time::Instant;

// cargo test -p ironcalc_base bench_incremental --release -- --ignored --nocapture
#[test]
#[ignore]
fn bench_incremental() {
    // #849's shape: many independent chains, so editing one chain's head
    // recomputes ~`length` cells incrementally vs `chains * length` fully.
    let chains = 400;
    let length = 100;
    let iters = 200;
    let build = |mode| {
        let mut m = Model::new_empty("m", "en", "UTC", "en")
            .unwrap()
            .with_recalc_mode(mode);
        for col in 1..=chains {
            let name = crate::expressions::utils::number_to_column(col).unwrap();
            m.set_user_input(0, 1, col, "1".into()).unwrap();
            for r in 2..=length {
                m.set_user_input(0, r, col, format!("={name}{}+1", r - 1))
                    .unwrap();
            }
        }
        m.evaluate(); // full pass builds the graph under the chosen mode
        m
    };

    let mut full = build(RecalcMode::Full);
    let t = Instant::now();
    for i in 0..iters {
        full.set_user_input(0, 1, 1, format!("{i}")).unwrap();
        full.evaluate();
    }
    let full_t = t.elapsed();

    let mut inc = build(RecalcMode::Incremental);
    let t = Instant::now();
    for i in 0..iters {
        inc.set_user_input(0, 1, 1, format!("{i}")).unwrap();
        inc.evaluate();
    }
    let inc_t = t.elapsed();

    // sanity: same result at the foot of the edited chain
    assert_eq!(
        full.get_formatted_cell_value(0, length, 1),
        inc.get_formatted_cell_value(0, length, 1)
    );
    let cells = chains * length;
    println!("\n[bench] {cells} cells ({chains} chains x {length}) x {iters} single-cell edits\n        full={full_t:?} incremental={inc_t:?} | speedup={:.1}x\n",
        full_t.as_secs_f64() / inc_t.as_secs_f64());
}

// cargo test -p ironcalc_base bench_structural_edit --release -- --ignored --nocapture
//
// Inserting a blank row near the foot of many independent chains shifts every
// row below the boundary but leaves the majority above untouched. The graph
// shifts its edges instead of rebuilding, so the recompute after the edit only
// revisits the shifted rows; a full pass redoes every cell. This times the
// recompute alone (the displacement itself costs the same in both modes).
#[test]
#[ignore]
fn bench_structural_edit() {
    let chains = 400;
    let length = 200;
    let build = |mode| {
        let mut m = Model::new_empty("m", "en", "UTC", "en")
            .unwrap()
            .with_recalc_mode(mode);
        for col in 1..=chains {
            let name = crate::expressions::utils::number_to_column(col).unwrap();
            m.set_user_input(0, 1, col, "1".into()).unwrap();
            for r in 2..=length {
                m.set_user_input(0, r, col, format!("={name}{}+1", r - 1))
                    .unwrap();
            }
        }
        m.evaluate();
        m
    };
    let insert_at = length - 1; // near the bottom

    let mut full = build(RecalcMode::Full);
    full.insert_rows(0, insert_at, 1).unwrap();
    let t = Instant::now();
    full.evaluate();
    let full_t = t.elapsed();

    let mut inc = build(RecalcMode::Incremental);
    inc.insert_rows(0, insert_at, 1).unwrap();
    let t = Instant::now();
    inc.evaluate();
    let inc_t = t.elapsed();

    assert_eq!(
        full.get_formatted_cell_value(0, length + 1, 1),
        inc.get_formatted_cell_value(0, length + 1, 1)
    );
    let cells = chains * length;
    println!("\n[bench] insert 1 row near foot of {cells} cells ({chains} chains x {length})\n        recompute: full={full_t:?} incremental={inc_t:?} | speedup={:.1}x\n",
        full_t.as_secs_f64() / inc_t.as_secs_f64());
}
