#![no_main]

use betterlite_core::api::deepgram::transcription::response::decode_deepgram_transcript;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(decode_deepgram_transcript(data));
});
