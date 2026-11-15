#![no_main]

use betterlite_vertex::vertex_speech::audio::decode_vertex_speech_audio;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_vertex_speech_audio(bytes, &|_audio| None));
});
