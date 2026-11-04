#![no_main]
use betterlite_config::collector::{
    CollectorConfigInput, decode_collector_settings, parse_collector_address,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = decode_collector_settings(data) {
        match input {
            CollectorConfigInput::Settings { config } => {
                let _ = config.validate();
                let _ = parse_collector_address(&config.address);
            }
            CollectorConfigInput::Suppression { value } => {
                let _ = betterlite_env::flag(
                    "LITELLM_SUPPRESS_SPEND_LOG_TRACEBACKS",
                    value.as_deref().unwrap_or_default(),
                );
            }
        }
    }
    if let Ok(address) = std::str::from_utf8(data) {
        let _ = parse_collector_address(address);
    }
});
