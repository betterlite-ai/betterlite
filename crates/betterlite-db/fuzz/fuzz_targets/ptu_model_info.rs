#![no_main]

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::ptu::charges::decode::decode_ptu_model_info;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(value) = serde_json::from_slice::<JsonData>(bytes) {
        let _decoded = decode_ptu_model_info(&value);
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        let _decoded = decode_ptu_model_info(&JsonData::String(text.to_owned()));
    }
});
