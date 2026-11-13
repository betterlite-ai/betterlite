#![no_main]

use betterlite_core::diagnostic_snapshots::process::parse_proc_memory;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let Some((statm, meminfo)) = input.split_at_checked(input.len() >> 1) else {
        return;
    };
    let _outcome = parse_proc_memory(statm, meminfo, 4096);
});
