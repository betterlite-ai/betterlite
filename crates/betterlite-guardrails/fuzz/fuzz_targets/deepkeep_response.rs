#![no_main]

use betterlite_guardrails::deepkeep::{Inputs, Response};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _response = serde_json::from_slice::<Response>(bytes);
    let _inputs = serde_json::from_slice::<Inputs>(bytes);
});
