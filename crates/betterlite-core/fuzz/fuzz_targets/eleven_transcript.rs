#![no_main]

use betterlite_core::api::elevenlabs::transcription::response::decode_eleven_transcript;
use libfuzzer_sys::fuzz_target;

const MAX_INPUT: usize = 0x0004_0000;
// the production limit is 16 MiB; this one fits inside MAX_INPUT so the over-limit branch is reachable
const LIMIT: usize = 0x0001_0000;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > MAX_INPUT {
        return;
    }
    drop(decode_eleven_transcript(bytes, LIMIT));
});
