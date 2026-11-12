#![no_main]

use betterlite_openai::api::batch_records::{WireDecodeLimits, decode_batch_records};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let limits = WireDecodeLimits {
        max_json_bytes: 0x0010_0000,
        max_event_bytes: 0x0001_0000,
        max_items: 10_000,
    };
    let _decoded = decode_batch_records(bytes, &limits);
});
