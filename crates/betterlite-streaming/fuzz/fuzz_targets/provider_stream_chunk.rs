#![no_main]

use betterlite_streaming::provider_chunk_decode::fuzz_input::decode_fuzzed_stream_chunk;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_fuzzed_stream_chunk(bytes);
});
