#![no_main]

use betterlite_vertex::legacy_prediction::frames::PredictionFrames;
use betterlite_vertex::legacy_prediction::records::decode_legacy_prediction;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_legacy_prediction(bytes);
    let mut frames = PredictionFrames::default();
    let _pushed = frames.push(bytes);
});
