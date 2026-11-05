#![no_main]

use betterlite_openai::files::records::decode_file_object;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_file_object(bytes);
});
