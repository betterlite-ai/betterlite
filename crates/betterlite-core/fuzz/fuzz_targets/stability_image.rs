#![no_main]

use betterlite_core::api::stability::generation::reply::decode_stability_image;
use betterlite_openai::api::image_contracts::records::{
    ImageHeaders, ImageHttpResponse, ImageResponse,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let response = ImageHttpResponse {
        status_code: 200,
        headers: ImageHeaders::new(),
        body: bytes.to_vec(),
    };
    drop(decode_stability_image(
        &response,
        &mut ImageResponse::default(),
    ));
});
