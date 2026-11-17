#![no_main]

use betterlite_anthropic::response::decode_anthropic_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _outcome = decode_anthropic_reply(data);
});
