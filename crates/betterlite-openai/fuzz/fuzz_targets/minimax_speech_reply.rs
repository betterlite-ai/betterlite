#![no_main]

use betterlite_openai::minimax_speech::reply::{SpeechReply, decode_speech_reply};
use http::HeaderMap;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let headers = HeaderMap::new();
    let _reply = decode_speech_reply(SpeechReply {
        status: 200,
        headers: &headers,
        body: bytes,
    });
});
