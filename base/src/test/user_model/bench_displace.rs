#![allow(clippy::unwrap_used)]
#![allow(clippy::print_stdout)]
use crate::Model;
use std::time::Instant;

// cargo test -p ironcalc_base bench_insert_rows_displace --release -- --ignored --nocapture
//
// Measures the cost of a structural edit: insert_rows shifts the references in
// every formula in the workbook. The in-place AST path rewrites each formula
// directly instead of rendering it to a string and reparsing it.
#[test]
#[ignore]
fn bench_insert_rows_displace() {
    let cols = 30;
    let rows = 2000;
    let mut m = Model::new_empty("m", "en", "UTC", "en").unwrap();
    for col in 1..=cols {
        let name = crate::expressions::utils::number_to_column(col).unwrap();
        m.set_user_input(0, 1, col, "1".into()).unwrap();
        for r in 2..=rows {
            m.set_user_input(0, r, col, format!("={name}{}+1", r - 1))
                .unwrap();
        }
    }
    m.evaluate();
    let formulas = cols * (rows - 1);

    let t = Instant::now();
    m.insert_rows(0, 1, 1).unwrap();
    let elapsed = t.elapsed();

    println!("\n[bench] insert_rows over {formulas} formulas: {elapsed:?}\n");
}
