#![no_main]

use betterlite_models::entry::ModelFields;
use libfuzzer_sys::fuzz_target;
use serde_json::value::RawValue;
use std::str;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(source) = str::from_utf8(bytes)
        && let Ok(raw) = serde_json::from_str::<Box<RawValue>>(source)
    {
        let _outcome = ModelFields::parse(raw.as_ref());
    }
});
