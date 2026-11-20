#![no_main]

use betterlite_openai::edenai_common::{parse_model_catalog, reported_cost_from_bytes};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _cost = reported_cost_from_bytes(bytes);
    let _catalog = parse_model_catalog(bytes);
});
