#![no_main]

use betterlite_http::proxy::UpstreamResponse;
use betterlite_vertex::vertex_recognition::response::decode_vertex_recognition;
use http::header::HeaderMap;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let reply = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: bytes.to_vec(),
    };
    drop(decode_vertex_recognition(&reply));
});
