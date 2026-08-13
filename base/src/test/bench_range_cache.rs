#![allow(clippy::unwrap_used)]
#![allow(clippy::print_stdout)]
use crate::Model;
use std::time::Instant;

// cargo test -p ironcalc_base bench_range_cache --release -- --ignored --nocapture
//
// Many formulas aggregate the same range in one pass. With the per-pass cache
// the range is materialized once; without it every formula re-evaluates every
// cell in the range. Runs identically on origin/main (no cache) and this branch,
// so the wall-clock ratio is the cache's effect on this workload.
#[test]
#[ignore]
fn bench_range_cache() {
    let range_rows = 2000; // cells in the shared range
    let formulas = 1000; // formulas aggregating it
    let passes = 10; // full evaluations timed

    let mut model = Model::new_empty("m", "en", "UTC", "en").unwrap();
    for row in 1..=range_rows {
        model.set_user_input(0, row, 1, "1".into()).unwrap(); // A1:A{range_rows}
    }
    for i in 0..formulas {
        // C{i} = SUM(A1:A{range_rows}) — all share one range.
        model
            .set_user_input(0, i + 1, 3, format!("=SUM(A1:A{range_rows})"))
            .unwrap();
    }

    let t = Instant::now();
    for _ in 0..passes {
        model.evaluate();
    }
    let elapsed = t.elapsed();

    assert_eq!(
        model.get_formatted_cell_value(0, 1, 3),
        Ok(range_rows.to_string())
    );
    let reads = (range_rows as u64) * (formulas as u64) * (passes as u64);
    println!(
        "\n[bench] {formulas} SUM over A1:A{range_rows}, {passes} passes ({reads} logical cell reads)\n        total={elapsed:?} per_pass={:?}\n",
        elapsed / passes as u32
    );
}
