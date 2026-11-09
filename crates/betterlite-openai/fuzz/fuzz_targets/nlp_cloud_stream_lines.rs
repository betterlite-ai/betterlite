#![no_main]

use std::str;

use betterlite_openai::nlp_cloud_chat::stream::{
    StreamError, StreamEvent, clean_and_iterate_chunks,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = str::from_utf8(bytes) else {
        return;
    };
    let cleaned = clean_and_iterate_chunks(text.split('\n'));
    let _decoded: Vec<_> = cleaned
        .iter()
        .map(|line| {
            serde_json::from_str::<StreamEvent>(line)
                .map_err(|source| StreamError::Malformed { source })
        })
        .collect();
});
