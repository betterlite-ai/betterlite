#![no_main]
use betterlite_config::sections::compatible_image_credentials::Host;
use betterlite_openai::api::image_contracts::records::{
    ImageHeaders, ImageHttpResponse, ImageResponse,
};
use betterlite_openai::compatible_image_hosts::reply::decode_compatible_image_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let reply = ImageHttpResponse {
        status_code: 200,
        headers: ImageHeaders::new(),
        body: bytes.to_vec(),
    };
    for host in [Host::Aiml, Host::CometApi] {
        let _decoded = decode_compatible_image_reply(host, &reply, ImageResponse::default());
    }
});
