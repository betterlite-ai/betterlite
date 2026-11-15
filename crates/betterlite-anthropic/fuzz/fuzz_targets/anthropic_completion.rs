#![no_main]

use betterlite_anthropic::completion::reply::decode_legacy_anthropic_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _decoded = decode_legacy_anthropic_reply(502, input);
});
