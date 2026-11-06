#![no_main]

use betterlite_openai::files::records::{FileObject, decode_file_object};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_file_object::<FileObject>(200, bytes);
    let _failed = decode_file_object::<FileObject>(400, bytes);
});
