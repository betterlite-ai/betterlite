#![no_main]

use betterlite_a2a::agent_card::merge::normalize_protocol_version;
use betterlite_core::agents::json::AgentJson;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = serde_json::from_slice::<AgentJson>(bytes)
        .ok()
        .and_then(|value| normalize_protocol_version(&value));
});
