#![no_main]
use betterlite_config::collector::{
    CollectorConfigInput, decode_collector_settings, load_spend_traceback_policy,
    parse_collector_address,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = decode_collector_settings(data) {
        match input {
            CollectorConfigInput::Settings { config } => {
                let _address = parse_collector_address(&config.address);
            }
            CollectorConfigInput::Address { value } => {
                let _address = parse_collector_address(&value);
            }
            CollectorConfigInput::Suppression { value } => {
                let _policy = load_spend_traceback_policy(value.as_deref());
            }
        }
    }
});
