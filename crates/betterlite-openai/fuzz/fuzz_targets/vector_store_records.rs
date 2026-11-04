#![no_main]

use betterlite_openai::api::vector_store_records::{
    VectorStoreRecordKind, WireDecodeLimits, decode_vector_store_records,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let limits = WireDecodeLimits {
        max_json_bytes: 1_048_576,
        max_event_bytes: 262_144,
        max_items: 10_000,
    };
    if let Some((&kind, json)) = bytes.split_first() {
        if let Ok(kind) = VectorStoreRecordKind::try_from(kind) {
            let _decoded = decode_vector_store_records(kind, json, limits);
        }
    }
});
