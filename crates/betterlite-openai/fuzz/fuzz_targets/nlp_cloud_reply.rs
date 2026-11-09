#![no_main]

use betterlite_openai::nlp_cloud_chat::reply::{ReplyInput, decode_reply};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_reply(ReplyInput {
        status_code: 200,
        body: bytes,
        model: "fuzz",
        created: 0,
        id: String::from("fuzz"),
    });
});
