#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::str::from_utf8;

use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::private_json::safe_json::safe_json_loads;
use rstest::rstest;

use crate::config_override_support::decode::{decode_config_override, decode_stored_override};
use crate::config_override_support::models::OverrideKind;

#[rstest]
fn replays_the_config_override_corpus(#[files("fuzz/corpus/config_override/*")] path: PathBuf) {
    let bytes = fs::read(&path).unwrap();
    let kinds = [OverrideKind::HashicorpVault, OverrideKind::CyberArk];
    for kind in kinds {
        drop(decode_config_override(&bytes, kind));
    }
    if let Ok(text) = from_utf8(&bytes) {
        let stored = JsonValue::String(text.to_owned());
        for kind in kinds {
            drop(decode_stored_override(&stored, kind));
        }
    }
    if let Ok(parsed) = safe_json_loads(&bytes) {
        for kind in kinds {
            drop(decode_stored_override(&parsed, kind));
        }
    }
}
