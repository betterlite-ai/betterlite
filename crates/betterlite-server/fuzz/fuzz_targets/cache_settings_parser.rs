#![no_main]

use betterlite_server::cache_settings::parse::parse_stored_settings;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let _outcome = parse_stored_settings(&serde_json::Value::String(text.to_owned()));
});
