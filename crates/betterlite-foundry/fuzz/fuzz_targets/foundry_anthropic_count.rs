#![no_main]

use betterlite_foundry::anthropic_count::reply::decode_foundry_count_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_foundry_count_reply(bytes);
});
