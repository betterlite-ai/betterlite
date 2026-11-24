#![no_main]

use std::str::from_utf8;

use betterlite_config::sections::settings::JsonValue;
use betterlite_server::config_override_support::decode::{
    decode_config_override, decode_stored_override,
};
use betterlite_server::config_override_support::models::OverrideKind;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let kinds = [OverrideKind::HashicorpVault, OverrideKind::CyberArk];
    for kind in kinds {
        drop(decode_config_override(data, kind));
    }
    if let Ok(text) = from_utf8(data) {
        let stored = JsonValue::String(text.to_owned());
        for kind in kinds {
            drop(decode_stored_override(&stored, kind));
        }
    }
    if let Ok(value) = serde_json::from_slice::<JsonValue>(data) {
        for kind in kinds {
            drop(decode_stored_override(&value, kind));
        }
    }
});
