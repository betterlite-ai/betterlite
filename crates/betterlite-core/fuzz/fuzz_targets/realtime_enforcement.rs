#![no_main]

use betterlite_core::api::realtime_sender::enforce::enforce_session_update;
use libfuzzer_sys::fuzz_target;
use std::convert::Infallible;
use std::str;

fuzz_target!(|bytes: &[u8]| {
    let Ok(message) = str::from_utf8(bytes) else {
        return;
    };
    let mut transcription = false;
    let _result =
        enforce_session_update::<Infallible, Infallible>(message, "allowed", &mut transcription);
});
