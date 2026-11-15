#![no_main]

use betterlite_vertex::gemini_image_policy::decode::decode_image_options;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| drop(decode_image_options(bytes)));
