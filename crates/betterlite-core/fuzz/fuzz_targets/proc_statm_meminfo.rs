#![no_main]

use betterlite_core::diagnostic_snapshots::parse_proc_memory;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let split = input.len() / 2;
    let _ = parse_proc_memory(&input[..split], &input[split..], 4096);
});
