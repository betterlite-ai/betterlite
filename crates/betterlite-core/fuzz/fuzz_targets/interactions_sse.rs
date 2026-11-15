#![no_main]

use betterlite_core::api::interactions::streaming::base::parse_stream_chunk;
use betterlite_core::api::interactions::streaming::response::{
    InteractionBufferedResponse, InteractionsSyncResponse,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let response = InteractionBufferedResponse {
        headers: Default::default(),
        body: bytes.to_vec(),
    };
    for line in response.iter_lines().flatten() {
        let _parsed = parse_stream_chunk(&line);
    }
});
