#![no_main]

use betterlite_core::api::fal::bria::append_singular_image;
use betterlite_core::api::fal::images::decode::decode_fal_image_reply;
use betterlite_core::api::fal::images::decode_url_only_reply;
use betterlite_core::api::fal::recraft::append_url_images;
use betterlite_openai::api::image_contracts::records::{ImageHeaders, ImageResponse};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let headers = ImageHeaders::new();
    drop(decode_fal_image_reply(200, &headers, data));
    drop(decode_url_only_reply(200, &headers, data));
    let mut bria_response = ImageResponse::default();
    drop(append_singular_image(
        &mut bria_response,
        200,
        &headers,
        data,
    ));
    let mut recraft_response = ImageResponse::default();
    drop(append_url_images(
        &mut recraft_response,
        200,
        &headers,
        data,
    ));
});
