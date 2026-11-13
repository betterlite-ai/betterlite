#![no_main]

use betterlite_models::passthrough_policies::query_merge::decode_request_query;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|value: &str| {
    let _decoded = decode_request_query(value);
});
