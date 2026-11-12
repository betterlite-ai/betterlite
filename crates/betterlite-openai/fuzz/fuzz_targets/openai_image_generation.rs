#![no_main]
use betterlite_openai::image_generation::decode::decode_image_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _reply = decode_image_reply(bytes, 1_700_000_000);
});
