#![no_main]

use std::str;

use betterlite_core::api::search_providers::google_pse::decode_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(text) = str::from_utf8(bytes) {
        drop(decode_reply(text));
    }
});
