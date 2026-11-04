#![no_main]

use betterlite_openai::api::vector_store_records::wire::{
    VectorStoreRecordKind, WireDecodeLimits, decode_vector_store_records,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let limits = WireDecodeLimits {
        max_json_bytes: 0x0010_0000,
        max_event_bytes: 0x0004_0000,
        max_items: 10_000,
    };
    if let Some((&raw_kind, json)) = bytes.split_first()
        && let Ok(kind) = VectorStoreRecordKind::try_from(raw_kind)
    {
        let _decoded = decode_vector_store_records(kind, json, limits);
    }
});
