#![no_main]

use betterlite_logging::passthrough_stream_logging::lines::convert_raw_bytes_to_str_lines;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _lines = convert_raw_bytes_to_str_lines(&chunk_list(bytes));
});

// A stride taken from the front varies the chunk boundaries; the corpus replay splits a file the same way.
fn chunk_list(bytes: &[u8]) -> Vec<Vec<u8>> {
    let Some((&first, rest)) = bytes.split_first() else {
        return Vec::new();
    };
    let stride = usize::from(first).rem_euclid(7).saturating_add(1);
    rest.chunks(stride).map(<[u8]>::to_vec).collect()
}
