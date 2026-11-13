#![no_main]

use betterlite_vertex::google_native_content::response::decode_google_native_content;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_google_native_content(bytes));
});
