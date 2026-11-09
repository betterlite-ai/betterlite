#![no_main]

use betterlite_models::ptu_pricing::parse_ptu_instant;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|value: &str| {
    let _outcome = parse_ptu_instant(value);
});
