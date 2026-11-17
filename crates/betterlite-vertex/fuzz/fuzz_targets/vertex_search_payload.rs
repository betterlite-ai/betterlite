#![no_main]

use betterlite_vertex::vertex_discovery_search::records::payload::decode_vertex_search_payload;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _page = decode_vertex_search_payload(bytes, "query");
});
