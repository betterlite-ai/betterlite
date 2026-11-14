#![no_main]

use betterlite_core::api::fal::images::decode::decode_fal_image_reply;
use betterlite_openai::api::image_contracts::records::ImageHeaders;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(decode_fal_image_reply(200, &ImageHeaders::new(), data));
});
