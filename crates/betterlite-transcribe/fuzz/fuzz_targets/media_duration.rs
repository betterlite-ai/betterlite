#![no_main]

use std::io::Cursor;

use betterlite_transcribe::media_source::media_duration_seconds;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _seconds = media_duration_seconds(Cursor::new(bytes));
});
