#![no_main]

use betterlite_openai::api::audio_file_input::decoder::{AudioDecodeLimits, decode_audio_duration};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let limits = AudioDecodeLimits {
        max_bytes: 0x0010_0000,
        max_frames: 33_177_600_000,
        max_sample_rate: 384_000,
    };
    let _duration = decode_audio_duration(bytes, &limits);
});
