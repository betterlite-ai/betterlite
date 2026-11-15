#![no_main]

use betterlite_core::api::recraft::reply::decode_recraft_images;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageHttpResponse};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let response = ImageHttpResponse {
        status_code: 200,
        headers: ImageHeaders::new(),
        body: bytes.to_vec(),
    };
    drop(decode_recraft_images(&response));
});
