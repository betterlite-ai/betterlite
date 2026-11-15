#![no_main]

use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::ovhcloud_audio::reply::decode_transcription_reply as decode_ovhcloud_transcription;
use betterlite_openai::ovhcloud_chat::chunk::chunk_parser;
use betterlite_openai::ovhcloud_embedding::reply::decode_embedding_reply;
use betterlite_openai::scaleway_audio::reply::decode_transcription_reply as decode_scaleway_transcription;
use betterlite_prompts::arguments::json::Object;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let json_headers = Headers::from([(
        String::from("content-type"),
        String::from("application/json"),
    )]);
    drop(decode_embedding_reply(bytes, 200, &Headers::new()));
    drop(decode_ovhcloud_transcription(bytes, 200, &Headers::new()));
    drop(decode_scaleway_transcription(bytes, 200, &Headers::new()));
    drop(decode_scaleway_transcription(bytes, 200, &json_headers));
    drop(
        serde_json::from_slice::<Object>(bytes)
            .ok()
            .map(|chunk| chunk_parser(&chunk)),
    );
});
