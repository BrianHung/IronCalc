#![allow(clippy::unwrap_used)]
#![allow(clippy::print_stdout)]
use crate::{Model, RecalcMode};
use std::time::Instant;

// cargo test -p ironcalc_base bench_incremental --release -- --ignored --nocapture
#[test]
#[ignore]
fn bench_incremental() {
    // sondt's shape (#849): many independent dependency chains. Editing one
    // chain's head only affects that chain, so incremental recomputes ~`length`
    // cells while full recomputes all `chains * length`.
    let chains = 400;
    let length = 100;
    let iters = 200;
    let build = |mode| {
        let mut m = Model::new_empty("m", "en", "UTC", "en").unwrap();
        for col in 1..=chains {
            m.set_user_input(0, 1, col, "1".into()).unwrap();
            for r in 2..=length {
                let prev = crate::expressions::utils::number_to_column(col).unwrap();
                m.set_user_input(0, r, col, format!("={prev}{}+1", r - 1))
                    .unwrap();
            }
        }
        m.evaluate();
        m.set_recalc_mode(mode);
        m.evaluate(); // one full pass to build the graph under the new mode
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
