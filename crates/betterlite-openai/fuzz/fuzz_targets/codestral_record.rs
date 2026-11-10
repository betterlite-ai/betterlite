#![no_main]

use betterlite_openai::codestral_records::chunk::decode_codestral_chunk;
use betterlite_openai::codestral_records::reply::decode_codestral_reply;
use betterlite_openai::codestral_records::{RecordKind, RecordSource};
use libfuzzer_sys::fuzz_target;
use uuid::Uuid;

fuzz_target!(|bytes: &[u8]| {
    let source = RecordSource {
        id: Uuid::nil(),
        created: 0,
    };
    let Some((&tag, body)) = bytes.split_first() else {
        return;
    };
    match RecordKind::from_tag(tag) {
        Some(RecordKind::Reply) => {
            let _decoded = decode_codestral_reply(200, body, &source);
        }
        Some(RecordKind::Chunk) => {
            let _decoded = decode_codestral_chunk(body, &source);
        }
        None => {}
    }
});
