#![no_main]

use betterlite_openai::watsonx_audio::reply::decode_watsonx_transcription_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _reply = decode_watsonx_transcription_reply(bytes);
});
