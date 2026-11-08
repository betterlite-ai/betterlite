#![no_main]
use betterlite_openai::cohere_chat_v2::reply::{ReplyInput, ReplyTemplate, decode_cohere_chat_v2};
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_cohere_chat_v2(ReplyInput {
        body: bytes,
        status: 200,
        model: "command-r",
        created: 0,
        template: ReplyTemplate::default(),
    });
});
