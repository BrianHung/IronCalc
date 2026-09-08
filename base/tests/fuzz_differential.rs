//! Differential fuzz: identical random operation sequences on two independently
//! built models, comparing every cell after every `evaluate()`.
//!
//! Both sides are built by the same constructor, so the models differ in
//! nothing: same code, same input, same order. A divergence is therefore
//! nondeterminism inside the engine — an evaluation order that follows hash-map
//! iteration, a traversal that depends on insertion history, an accumulator with
//! no fixed association. The comparator is constructor-agnostic (see
//! `common::ModelKind`), so what a run means is decided by which two
//! constructors it is given, not by the harness.
//!
//! CI runs a bounded seed set (defaults: 8 seeds × 40 steps). Override with
//! `FUZZ_SEEDS`, `FUZZ_STEPS`, `FUZZ_START`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;

use common::*;
use std::collections::BTreeMap;

/// Generator classes parked by default because they reproduce bugs that are
/// already on `main` and that this test does not fix. Parking them here rather
/// than in the CI invocation keeps a bare `cargo test` green and keeps the list
/// next to the reason for it. Setting the matching env var — to a shorter list,
/// or to the empty string — searches them again, which is how each entry was
/// found and how it should be checked once its bug is fixed.
///
/// `ArrayFormula`: a CSE array displaced by a column insert materializes a
/// phantom cell one column past its extent on one model and not on the other,
/// and downstream that turns into a spurious `#SPILL!`. Two models built the
/// same way disagree about it, so the extent bookkeeping is reading something
/// order-dependent.
const DEFAULT_AVOID_OPS: &str = "ArrayFormula";

/// `SCELL` / `NCELL` / `NRANGE`: a formula that reads a defined name in scalar
/// context (`=SCELL+1`, or plain `=NRANGE` set before the name exists) panics
/// the engine outright — "Larger-than-1x1 array reached scalar-context cell" —
/// as soon as that name is bound or rebound to a multi-cell range. Two ops,
/// deterministic (seeds 298 and 390 both minimize to exactly this), and it
/// aborts the scenario before any comparison happens, so it hides whatever
/// else the seed would have searched.
const DEFAULT_AVOID_FORMULAS: &str = "SCELL,NCELL,NRANGE";

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn env_list(name: &str, default: &str) -> Vec<&'static str> {
    std::env::var(name)
        .unwrap_or_else(|_| default.to_string())
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| Box::leak(s.to_string().into_boxed_str()) as &'static str)
        .collect()
}

#[test]
fn differential_same_engine_twice() {
    install_quiet_hook();
    let seeds = env_usize("FUZZ_SEEDS", 8) as u64;
    let start = env_usize("FUZZ_START", 1) as u64;
    let steps = env_usize("FUZZ_STEPS", 40);
    let max_findings = env_usize("FUZZ_MAX_FINDINGS", 20);
    let no_minimize = std::env::var("FUZZ_NO_MINIMIZE").is_ok();
    let avoid_formulas = env_list("FUZZ_AVOID_FORMULAS", DEFAULT_AVOID_FORMULAS);
    let avoid_ops = env_list("FUZZ_AVOID_OPS", DEFAULT_AVOID_OPS);

    let mut findings: BTreeMap<String, (u64, Failure, Vec<Op>)> = BTreeMap::new();
    let mut total = Stats::default();
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
        let result = run_scenario(&ops);
        set_quiet(false);
        match result {
            Ok(stats) => {
                total.evaluates += stats.evaluates;
                total.ops_applied += stats.ops_applied;
                total.ops_rejected += stats.ops_rejected;
                total.reloads += stats.reloads;
            }
            Err(f) => {
                failing_seeds.push(seed);
                eprintln!("seed {seed}: FAIL {f}");
                let (min_ops, min_f) = if no_minimize {
                    (ops.clone(), f.clone())
                } else {
                    minimize(&ops, &f.kind)
                };
                // A nondeterminism finding is by nature intermittent: replaying
                // the same ops need not fail again. Tagging it says so, keeps a
                // flaky signature from being read as a stable wrong answer, and
                // tells the reader how strong the repro is.
                let clean = clean_runs(&min_ops, 6);
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
        "==== differential fuzz summary: seeds {}..{} x {steps} steps; evaluates={} ops_applied={} ops_rejected={} reloads={} failing_seeds={:?} avoid_formulas={avoid_formulas:?} avoid_ops={avoid_ops:?}",
        start,
        start + seeds - 1,
        total.evaluates,
        total.ops_applied,
        total.ops_rejected,
        total.reloads,
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
}
