//! Differential fuzz: identical random operation sequences on a Full model and an
//! Incremental model (plus a Verify model when `recalc_verify` is on), comparing
//! every cell after every `evaluate()`, and checking the incremental delta.
//!
//! CI runs a bounded seed set (defaults: 8 seeds × 40 steps). Override with
//! `FUZZ_SEEDS`, `FUZZ_STEPS`, `FUZZ_START`. The x86_64 job is `ubuntu-latest`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;

use common::*;
use std::collections::BTreeMap;

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_list(name: &str) -> Vec<&'static str> {
    std::env::var(name)
        .ok()
        .map(|v| {
            v.split(',')
                .filter(|s| !s.is_empty())
                .map(|s| Box::leak(s.to_string().into_boxed_str()) as &'static str)
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn differential_full_vs_incremental() {
    install_quiet_hook();
    let seeds = env_usize("FUZZ_SEEDS", 8) as u64;
    let start = env_usize("FUZZ_START", 1) as u64;
    let steps = env_usize("FUZZ_STEPS", 40);
    let max_findings = env_usize("FUZZ_MAX_FINDINGS", 20);
    let no_minimize = std::env::var("FUZZ_NO_MINIMIZE").is_ok();
    let check_verify = cfg!(feature = "recalc_verify") && std::env::var("FUZZ_NO_VERIFY").is_err();
    let avoid_formulas = env_list("FUZZ_AVOID_FORMULAS");
    let avoid_ops = env_list("FUZZ_AVOID_OPS");

    let mut findings: BTreeMap<String, (u64, Failure, Vec<Op>)> = BTreeMap::new();
    let mut total = Stats::default();
    // The delta-coverage floor is computed over the seeds that plant no
    // volatiles. A planted volatile re-rolls on every pass, so a volatile seed's
    // passes report `Everything` by construction; folding them into the
    // denominator measures `VOLATILE_SEED_EVERY` rather than incremental
    // coverage, and at enough seeds it drives the ratio under the floor on its
    // own (40 seeds x 40 steps landed at 662/1384).
    let mut floor = Stats::default();
    let mut failing_seeds = Vec::new();
    for seed in start..start + seeds {
        let cfg = GenConfig {
            steps,
            avoid_formulas: avoid_formulas.clone(),
            avoid_ops: avoid_ops.clone(),
            ..GenConfig::default()
        };
        let ops = generate(seed, cfg);
        set_quiet(true);
        let result = run_scenario(&ops, true, check_verify);
        set_quiet(false);
        match result {
            Ok(stats) => {
                total.evaluates += stats.evaluates;
                total.cells_deltas += stats.cells_deltas;
                total.everything_deltas += stats.everything_deltas;
                total.ops_applied += stats.ops_applied;
                total.ops_rejected += stats.ops_rejected;
                if !seed_plants_volatiles(seed) {
                    floor.evaluates += stats.evaluates;
                    floor.cells_deltas += stats.cells_deltas;
                }
            }
            Err(f) => {
                failing_seeds.push(seed);
                eprintln!("seed {seed}: FAIL {f}");
                let (min_ops, min_f) = if no_minimize {
                    (ops.clone(), f.clone())
                } else {
                    minimize(&ops, &f.kind, true, check_verify)
                };
                let clean = clean_runs(&min_ops, true, check_verify, 6);
                let sig = if clean > 0 {
                    format!("FLAKY({clean}/6 clean)|{}", signature(&min_f))
                } else {
                    signature(&min_f)
                };
                eprintln!(
                    "seed {seed}: minimized to {} ops, signature {sig}\n{min_f}\n{}",
                    min_ops.len(),
                    ops_to_rust(&min_ops)
                );
                findings.entry(sig).or_insert((seed, min_f, min_ops));
                if findings.len() >= max_findings {
                    break;
                }
            }
        }
    }
    let summary = format!(
        "==== differential fuzz summary: seeds {}..{} x {steps} steps; evaluates={} cells_deltas={} everything_deltas={} ops_applied={} ops_rejected={} non_volatile_evaluates={} non_volatile_cells_deltas={} failing_seeds={:?} avoid_formulas={avoid_formulas:?} avoid_ops={avoid_ops:?} verify={check_verify}",
        start,
        start + seeds - 1,
        total.evaluates,
        total.cells_deltas,
        total.everything_deltas,
        total.ops_applied,
        total.ops_rejected,
        floor.evaluates,
        floor.cells_deltas,
        failing_seeds
    );
    eprintln!("\n{summary}");
    if let Ok(path) = std::env::var("FUZZ_SUMMARY_FILE") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{summary}");
        }
    }
    if !findings.is_empty() {
        let mut report = String::new();
        for (sig, (seed, f, ops)) in &findings {
            report.push_str(&format!(
                "\n--- {sig} (seed {seed}, {} ops)\n{f}\n{}\n",
                ops.len(),
                ops_to_rust(ops)
            ));
        }
        panic!("{} distinct divergence(s):{report}", findings.len());
    }
    // A statistical guard, not a correctness one: short smoke runs (the 8x40
    // default, or a single structural-heavy seed) legitimately sit near 50%
    // because structural edits force Full passes, so only enforce the floor
    // on a sample large enough for the ratio to mean something. Volatile seeds
    // are out of the sample entirely; see `floor` above.
    if floor.evaluates >= 1000 && floor.cells_deltas < floor.evaluates / 2 {
        panic!(
            "fuzz Incremental coverage collapsed: cells_deltas={} of evaluates={} on the non-volatile seeds (need at least half Incremental)",
            floor.cells_deltas, floor.evaluates
        );
    }
}
