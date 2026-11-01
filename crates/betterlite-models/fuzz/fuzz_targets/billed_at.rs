#![no_main]

use betterlite_models::savings_evidence::billed_at::decode_billed_at;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|value: &str| {
    let _ = decode_billed_at(value);
});
