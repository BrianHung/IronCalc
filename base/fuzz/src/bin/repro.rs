//! Replays a libFuzzer crash artifact through the shared harness (with working
//! catch_unwind, unlike under libFuzzer's abort hook), prints the decoded Op
//! list, the failure, and a minimized Op-list repro.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ironcalc_base_fuzz::common::{
    clean_runs, install_quiet_hook, minimize, ops_to_rust, run_scenario, signature,
};
use ironcalc_base_fuzz::decode;

fn main() {
    install_quiet_hook();
    let verify = cfg!(feature = "recalc_verify");
    for path in std::env::args().skip(1) {
        let data = std::fs::read(&path).expect("read artifact");
        let Some(ops) = decode(&data) else {
            println!("{path}: input too short, no scenario");
            continue;
        };
        println!("==== {path}: {} ops", ops.len());
        match run_scenario(&ops, true, verify) {
            Ok(stats) => println!("{path}: CLEAN ({stats:?})"),
            Err(f) => {
                println!("{path}: FAIL {f}\nsignature: {}", signature(&f));
                let (min_ops, min_f) = minimize(&ops, &f.kind, true, verify);
                let clean = clean_runs(&min_ops, true, verify, 6);
                println!(
                    "minimized to {} ops (clean_runs {clean}/6)\n{min_f}\nops:\n{}",
                    min_ops.len(),
                    ops_to_rust(&min_ops)
                );
            }
        }
    }
}
