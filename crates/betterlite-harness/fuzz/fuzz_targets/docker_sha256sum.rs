#![no_main]
use betterlite_harness::sandbox::docker::snapshot::parse_sha256sum;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    let _snapshot = parse_sha256sum(data);
});
