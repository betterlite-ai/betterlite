#![no_main]

use betterlite_oracle::http_scenarios::decode_http_scenario;
use betterlite_oracle::http_scenarios::model::Limits;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_http_scenario(bytes, &Limits::default());
});
