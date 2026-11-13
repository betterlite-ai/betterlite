#![no_main]

use std::str;

use betterlite_openai::nlp_cloud_chat::stream::{StreamCleaner, decode_stream_line};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = str::from_utf8(bytes) else {
        return;
    };
    let mut cleaner = StreamCleaner::default();
    let mut cleaned: Vec<String> = text
        .split('\n')
        .filter_map(|line| cleaner.push(line).ok().flatten())
        .collect();
    if let Some(remainder) = cleaner.finish() {
        cleaned.push(remainder);
    }
    let _decoded: Vec<_> = cleaned
        .iter()
        .map(String::as_str)
        .map(decode_stream_line)
        .collect();
});
