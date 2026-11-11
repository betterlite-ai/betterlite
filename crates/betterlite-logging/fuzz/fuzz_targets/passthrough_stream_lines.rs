#![no_main]

use betterlite_logging::passthrough_stream_logging::lines::convert_raw_bytes_to_str_lines;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _lines = convert_raw_bytes_to_str_lines(&chunk_list(bytes));
});

// A width taken from the front varies the chunk boundaries, so a multibyte character can straddle two chunks.
fn chunk_list(bytes: &[u8]) -> Vec<Vec<u8>> {
    let Some((&width, rest)) = bytes.split_first() else {
        return Vec::new();
    };
    let width = usize::from(width).rem_euclid(7).saturating_add(1);
    rest.chunks(width).map(<[u8]>::to_vec).collect()
}
