#![no_main]

use std::collections::VecDeque;

use betterlite_core::api::interactions::streaming::base::parse_stream_chunk;
use betterlite_core::api::interactions::streaming::response::MAX_LINE_BYTES;
use betterlite_openai::aleph_alpha::lines::LineDecoder;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let mut decoder = LineDecoder::new(MAX_LINE_BYTES);
    let mut pending = VecDeque::new();
    if decoder.push(bytes, &mut pending).is_err() {
        return;
    }
    decoder.flush(&mut pending);
    for line in pending {
        drop(parse_stream_chunk(&line));
    }
});
