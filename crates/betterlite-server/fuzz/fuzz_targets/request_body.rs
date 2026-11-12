#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_server::api::request_body::decode_request_body;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    let _outcome = decode_request_body(body, &ApiLimits::default());
});
