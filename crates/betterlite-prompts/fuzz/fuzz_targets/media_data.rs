#![no_main]

use betterlite_prompts::media::base64_string::is_base64_encoded;
use betterlite_prompts::media::data_url::{MAX_MEDIA_BYTES, decode_data_url};
use libfuzzer_sys::fuzz_target;
use std::hint::black_box;
use std::str;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = str::from_utf8(bytes) else {
        return;
    };
    drop(decode_data_url(text, MAX_MEDIA_BYTES));
    black_box(is_base64_encoded(text));
});
