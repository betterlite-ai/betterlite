#![no_main]

use betterlite_vertex::provider_metadata::timestamp::LocalTimeContext;
use betterlite_vertex::vertex_rag_store::records::create::decode_vertex_rag_corpus;
use betterlite_vertex::vertex_rag_store::records::payload::decode_vertex_rag_payload;
use jiff::tz::TimeZone;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let local = LocalTimeContext {
        zone: TimeZone::UTC,
    };
    let _search = decode_vertex_rag_payload(bytes, "query");
    let _corpus = decode_vertex_rag_corpus(bytes, &local);
});
