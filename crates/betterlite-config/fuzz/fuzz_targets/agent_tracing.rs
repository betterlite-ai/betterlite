#![no_main]
use betterlite_config::sections::agent_tracing::decode_agent_tracing;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let _settings = decode_agent_tracing(bytes);
});
