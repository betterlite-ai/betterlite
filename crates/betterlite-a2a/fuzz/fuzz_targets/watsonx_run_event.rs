#![no_main]

use betterlite_a2a::watsonx_run::accumulator::{LineAccumulator, accumulate_text};
use betterlite_a2a::watsonx_run::event::RunEvent;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _event = serde_json::from_slice::<RunEvent>(bytes);
    let mut accumulator = LineAccumulator::default();
    let mut lines = Vec::new();
    for chunk in bytes.chunks(3) {
        if let Ok(mut complete) = accumulator.push(chunk) {
            lines.append(&mut complete);
        }
    }
    if let Some(line) = accumulator.finish() {
        lines.push(line);
    }
    let _text = accumulate_text(&lines);
});
