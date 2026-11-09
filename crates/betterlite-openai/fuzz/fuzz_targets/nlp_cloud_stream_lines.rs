#![no_main]

use std::str;

use betterlite_openai::nlp_cloud_chat::stream::{clean_and_iterate_chunks, decode_stream_line};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = str::from_utf8(bytes) else {
        return;
    };
    let cleaned = clean_and_iterate_chunks(text.split('\n'));
    let _decoded: Vec<_> = cleaned
        .iter()
        .map(|line| decode_stream_line(line))
        .collect();
});
