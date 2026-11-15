#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::soniox::prepare::decode::{decode_soniox_reply, identified_resource};
use betterlite_core::api::soniox::prepare::raise::provider_message;
use betterlite_core::api::soniox::prepare::response::{transcript, transcription_meta};
use betterlite_openai::openai_base_config::Headers;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let limits = ApiLimits::default();
    let Ok(reply) = decode_soniox_reply(200, Headers::new(), data, &limits) else {
        return;
    };
    drop(identified_resource(&reply));
    drop(transcription_meta(&reply));
    drop(transcript(&reply, &limits));
    drop(provider_message(&reply));
});
