#![no_main]

use betterlite_openai::bytez_wire::messages::RawMessage;
use betterlite_openai::bytez_wire::reply::{ReplyInput, decode_bytez_reply};
use http::HeaderMap;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let messages = [RawMessage {
        role: None,
        content: None,
    }];
    let _reply = decode_bytez_reply(&ReplyInput {
        body: bytes,
        status: 200,
        model: "org/model",
        created: 1,
        messages: &messages,
        headers: &HeaderMap::new(),
    });
});
