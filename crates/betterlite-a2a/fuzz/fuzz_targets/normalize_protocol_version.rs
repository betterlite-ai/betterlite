#![no_main]

use betterlite_a2a::agent_card::normalize_protocol_version_bytes;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = normalize_protocol_version_bytes(bytes);
});
