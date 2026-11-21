#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_server::api::assistant_dispatch::body::decode_assistant_body;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    let _decoded = decode_assistant_body(body, &ApiLimits::default());
});
