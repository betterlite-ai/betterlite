#![no_main]
#![expect(
    clippy::disallowed_types,
    reason = "fuzzes the stored cache_settings parser, which takes a free-form JSON value"
)]

use betterlite_server::cache_settings::parse::parse_stored_settings;
use libfuzzer_sys::fuzz_target;
use serde_json::Value;

fuzz_target!(|text: &str| {
    let _ = parse_stored_settings(&Value::String(text.to_owned()));
});
