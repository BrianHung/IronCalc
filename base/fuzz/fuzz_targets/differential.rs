#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    ironcalc_base_fuzz::run_bytes(data);
});
