#![no_main]

use betterlite_core::api::bfl::common::{check_bfl_poll_url, decode_bfl_reply};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(decode_bfl_reply(data));
    if let Ok(text) = std::str::from_utf8(data) {
        drop(check_bfl_poll_url(text));
    }
});
