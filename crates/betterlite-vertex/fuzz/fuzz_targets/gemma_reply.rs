#![no_main]

use betterlite_vertex::gemma_call::reply::decode_gemma_reply;
use libfuzzer_sys::fuzz_target;
use uuid::Uuid;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_gemma_reply(bytes, Uuid::nil, || 0));
});
