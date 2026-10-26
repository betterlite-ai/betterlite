#![no_main]

use betterlite_oracle::multipart_oracle::capture::{
    decode_capture_input, decode_multipart_capture,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = decode_multipart_capture("multipart/form-data", bytes);
    let _ = decode_capture_input(bytes);
});
