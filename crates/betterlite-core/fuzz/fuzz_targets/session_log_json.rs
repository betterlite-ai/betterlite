#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_session_history::decode::{
    SessionLogJsonKind, decode_session_log_json,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((selector, stored)) = data.split_first() else {
        return;
    };
    let kind = SessionLogJsonKind::from_selector(*selector);
    let _decoded = decode_session_log_json(stored, kind, ApiLimits::default());
});
