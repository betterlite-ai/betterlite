#![no_main]

use betterlite_openai::transcription::decode::{ReplyFormat, decode_transcription_reply};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = decode_transcription_reply(bytes, Some("application/json"), ReplyFormat::Json);
    let _ = decode_transcription_reply(bytes, Some("text/plain"), ReplyFormat::Text);
});
