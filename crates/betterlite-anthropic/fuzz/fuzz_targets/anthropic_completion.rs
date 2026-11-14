#![no_main]

use betterlite_anthropic::completion::reply::decode_legacy_anthropic_reply;
use betterlite_anthropic::completion::stream::decode_stream_line;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _decoded = decode_legacy_anthropic_reply(502, input);
    if let Ok(line) = str::from_utf8(input) {
        let _chunk = decode_stream_line(line);
    }
});
