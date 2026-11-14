#![no_main]

use betterlite_guardrails::noma_scan_policy::wire::ScanResponse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _response = ScanResponse::from_body(bytes);
});
