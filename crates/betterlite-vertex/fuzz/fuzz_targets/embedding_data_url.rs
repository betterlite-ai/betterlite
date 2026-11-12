#![no_main]

use std::str::from_utf8;

use betterlite_vertex::gemini_embedding_records::data_url::decode_embedding_data_url;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(source) = from_utf8(bytes) {
        drop(decode_embedding_data_url(source));
    }
});
