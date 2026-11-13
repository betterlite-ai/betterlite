#![no_main]

use betterlite_openai::modelscope_image::reply::{ImageReply, decode_image_generation_reply};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _reply = decode_image_generation_reply(
        ImageReply {
            status: 200,
            body: bytes,
        },
        1_700_000_000,
    );
});
