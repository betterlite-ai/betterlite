#![no_main]

use betterlite_core::api::fal::vision_chat::reply::decode_fal_chat;
use betterlite_openai::api::image_contracts::records::ImageHeaders;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(decode_fal_chat(&ImageHeaders::new(), data));
});
