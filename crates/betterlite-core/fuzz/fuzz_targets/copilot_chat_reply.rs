#![no_main]

use betterlite_core::api::copilot_chat::reply::transform_response;
use betterlite_http::proxy::UpstreamResponse;
use http::HeaderMap;
use libfuzzer_sys::fuzz_target;
use uuid::Uuid;

const MAX_INPUT: usize = 0x0004_0000;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > MAX_INPUT {
        return;
    }
    let response = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: bytes.to_vec(),
    };
    drop(transform_response(response, None, false, Uuid::new_v4));
});
