#!/bin/zsh
source "$HOME/.cargo/env" 2>/dev/null
cd /private/tmp/fix93
LOG=/private/tmp/fix93/verify_results.log
: > $LOG
run() {
  echo "\n===== $1 =====" >> $LOG
  shift
  "$@" >> $LOG 2>&1
  echo "EXIT: $?" >> $LOG
}
run "FULL lib+tests" cargo test -p ironcalc_base
run "INCREMENTAL lib" env IRONCALC_RECALC=incremental cargo test -p ironcalc_base --lib
run "VERIFY lib" env IRONCALC_RECALC=verify cargo test -p ironcalc_base --features recalc_verify --lib
run "CLIPPY" cargo clippy -p ironcalc_base --all-targets --all-features -- -D warnings
run "FUZZ structural-inclusive" env FUZZ_SEEDS=40 FUZZ_STEPS=150 cargo test -p ironcalc_base --test fuzz_differential -- --nocapture
echo "\nALL DONE" >> $LOG
