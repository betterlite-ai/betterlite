#![no_main]

use std::str;

use betterlite_core::api::video_records::helpers::{
    get_custom_provider_from_data, video_reference_to_id,
};
use betterlite_models::decisions::Json;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = str::from_utf8(bytes) else {
        return;
    };
    drop(video_reference_to_id(&Json::Text(text.to_owned())));
    let Ok(value) = serde_json::from_str::<Json>(text) else {
        return;
    };
    drop(video_reference_to_id(&value));
    if let Json::Object(fields) = &value {
        drop(get_custom_provider_from_data(fields));
    }
});
