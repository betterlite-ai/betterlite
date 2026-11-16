#![no_main]
use betterlite_openai::openrouter_image::decode::decode_openrouter_image_reply;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let _reply = decode_openrouter_image_reply(bytes);
});
