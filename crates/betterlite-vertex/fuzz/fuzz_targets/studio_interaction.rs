#![no_main]

use betterlite_vertex::studio_interactions::decode::{RawResponse, decode_studio_interaction};
use betterlite_vertex::studio_interactions::records::{InteractionRecord, StreamingRecord};
use indexmap::IndexMap;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let headers = IndexMap::new();
    let response = RawResponse {
        status: 200,
        headers: &headers,
        body: bytes,
    };
    let _interaction = decode_studio_interaction::<InteractionRecord>(&response);
    let _chunk = decode_studio_interaction::<StreamingRecord>(&response);
});
