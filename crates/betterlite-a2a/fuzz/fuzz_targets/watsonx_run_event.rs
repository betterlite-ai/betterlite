#![no_main]

use betterlite_a2a::watsonx_run::accumulator::{LineAccumulator, accumulate_text};
use betterlite_a2a::watsonx_run::event::RunEvent;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _event = serde_json::from_slice::<RunEvent>(bytes);
    let mut accumulator = LineAccumulator::default();
    let mut lines = Vec::new();
    for chunk in bytes.chunks(3) {
        lines.extend(accumulator.push(chunk).unwrap_or_default());
    }
    lines.extend(accumulator.finish());
    let _text = accumulate_text(&lines);
});
