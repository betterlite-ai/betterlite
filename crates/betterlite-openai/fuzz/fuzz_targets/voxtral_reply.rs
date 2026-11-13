#![no_main]

use betterlite_http::proxy::UpstreamResponse;
use betterlite_openai::api::speech_provider_contract::TextToSpeechRequestData;
use betterlite_openai::mistral_audio::speech::decode_voxtral_reply;
use betterlite_openai::mistral_audio::transcription::decode_voxtral_transcription_reply;
use betterlite_openai::openai_base_config::Object;
use http::HeaderMap;
use libfuzzer_sys::fuzz_target;
use serde_json::json;

fuzz_target!(|bytes: &[u8]| {
    let response = UpstreamResponse {
        status: 502,
        headers: HeaderMap::new(),
        body: bytes.to_vec(),
    };
    let mut body = Object::new();
    body.insert(String::from("response_format"), json!("mp3"));
    let request = TextToSpeechRequestData {
        dict_body: Some(body),
        ssml_body: None,
        headers: None,
    };
    drop(decode_voxtral_reply(&request, &response));
    drop(decode_voxtral_transcription_reply(&response));
});
