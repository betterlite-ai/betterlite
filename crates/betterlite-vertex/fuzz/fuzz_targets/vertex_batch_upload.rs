#![no_main]

use std::convert::Infallible;

use betterlite_vertex::vertex_batch_upload::line::decode_openai_batch_line;
use betterlite_vertex::vertex_batch_upload::rows::embeddings::rows;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(Some(line)) = decode_openai_batch_line(bytes) {
        drop(rows::<Infallible>(&line));
    }
});
