#![no_main]

use betterlite_openai::passthrough::transformation::convert_raw_bytes_to_lines;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let whole = convert_raw_bytes_to_lines(&[bytes]);
    let chunks: Vec<&[u8]> = bytes.chunks(3).collect();
    let split = convert_raw_bytes_to_lines(&chunks);
    assert_eq!(whole, split);
});
