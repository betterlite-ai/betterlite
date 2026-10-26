#![no_main]
use betterlite_harness::snapshots::unified_diff;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|text: &str| {
    let (old, new) = text.split_once('\0').unwrap_or((text, ""));
    let _ = unified_diff("fuzz.txt", Some(old), Some(new));
});
