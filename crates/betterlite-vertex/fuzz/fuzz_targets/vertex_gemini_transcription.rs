#![no_main]

use betterlite_http::proxy::UpstreamResponse;
// ast-grep-ignore: no-long-names
use betterlite_vertex::vertex_gemini_transcription::response::decode_vertex_gemini_transcription;
use http::header::HeaderMap;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let reply = UpstreamResponse {
        status: 200,
        headers: HeaderMap::new(),
        body: bytes.to_vec(),
    };
    // ast-grep-ignore: no-long-names
    drop(decode_vertex_gemini_transcription(&reply));
});
