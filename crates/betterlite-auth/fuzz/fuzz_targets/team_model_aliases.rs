#![no_main]

use std::str;

use betterlite_auth::identity_grants::team_grants::decode_model_aliases;
use betterlite_db::team_rows::aliases::ModelAliases;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(text) = str::from_utf8(bytes) {
        drop(decode_model_aliases(&ModelAliases::Text(text.to_owned())));
    }
    if let Ok(column) = serde_json::from_slice::<ModelAliases>(bytes) {
        drop(decode_model_aliases(&column));
    }
});
