#![no_main]

use betterlite_harness::providers::claude_code::stream::{ClaudeCodeStreamState, transform};
use betterlite_harness::types::JsonObject;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|line: &[u8]| {
    if let Ok(object) = serde_json::from_slice::<JsonObject>(line) {
        let mut state = ClaudeCodeStreamState::default();
        let _ = transform(&object, &mut state);
    }
});
