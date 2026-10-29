#![no_main]

use betterlite_models::price_fields::PriceFields;
use libfuzzer_sys::fuzz_target;
use serde_json::value::RawValue;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(source) = std::str::from_utf8(bytes) {
        if let Ok(raw) = serde_json::from_str::<Box<RawValue>>(source) {
            let _ = PriceFields::parse(raw.as_ref());
        }
    }
});
