#![no_main]

use betterlite_oracle::multipart_oracle::capture::{decode_multipart_capture, split_capture_input};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok((content_type, body)) = split_capture_input(bytes) {
        let _ = decode_multipart_capture(content_type, body);
    }
});
