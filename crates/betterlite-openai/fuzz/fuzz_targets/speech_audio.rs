#![no_main]

use std::str::from_utf8;

use betterlite_openai::api::audio_file_input::decoder::AudioDecodeLimits;
use betterlite_openai::api::speech_chat_bridge::audio_base64::decode_chat_audio_base64;
use betterlite_openai::api::speech_chat_bridge::media_type::sniff_speech_audio;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Some((&selector, input)) = bytes.split_first() else {
        return;
    };
    if selector == 0 {
        let Ok(text) = from_utf8(input) else {
            return;
        };
        let _decoded = decode_chat_audio_base64(text);
        return;
    }
    let _media_type = sniff_speech_audio(input, &AudioDecodeLimits::default());
});
