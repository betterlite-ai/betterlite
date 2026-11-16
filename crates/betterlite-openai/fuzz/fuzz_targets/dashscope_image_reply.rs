#![no_main]

use betterlite_openai::dashscope_image::reply::decode_dashscope_image_reply;
use betterlite_openai::openai_base_config::Headers;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_dashscope_image_reply(bytes, 200, &Headers::new());
});
