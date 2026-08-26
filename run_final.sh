#!/bin/zsh
source "$HOME/.cargo/env" 2>/dev/null
cd /private/tmp/fix90
LOG=/private/tmp/fix90/final_results.log
: > $LOG
run() { echo "\n===== $1 =====" >> $LOG; shift; "$@" >> $LOG 2>&1; echo "EXIT: $?" >> $LOG; }
run "FULL all" cargo test -p ironcalc_base
run "INCR lib" env IRONCALC_RECALC=incremental cargo test -p ironcalc_base --lib
run "VERIFY lib" env IRONCALC_RECALC=verify cargo test -p ironcalc_base --features recalc_verify --lib
run "CLIPPY" cargo clippy -p ironcalc_base --all-targets --all-features -- -D warnings
run "FMT" cargo fmt --check -p ironcalc_base
run "XLSX" cargo test -p ironcalc
run "X86 lib" cargo test -p ironcalc_base --lib --target x86_64-apple-darwin
run "SOAK 400x400" env FUZZ_SEEDS=400 FUZZ_STEPS=400 cargo test -p ironcalc_base --test fuzz_differential -- --nocapture
run "SOAK2 401-800" env FUZZ_START=401 FUZZ_SEEDS=400 FUZZ_STEPS=400 cargo test -p ironcalc_base --test fuzz_differential -- --nocapture
run "VERIFY-FUZZ 60x200" env FUZZ_SEEDS=60 FUZZ_STEPS=200 cargo test -p ironcalc_base --features recalc_verify --test fuzz_differential -- --nocapture
(cd bindings/wasm && make tests > /tmp/wasm_final.log 2>&1; echo "\n===== WASM =====\nEXIT: $?" >> $LOG)
echo "\nALL DONE" >> $LOG
