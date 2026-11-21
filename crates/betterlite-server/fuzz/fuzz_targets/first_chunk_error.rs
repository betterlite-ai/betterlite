#![no_main]

use betterlite_server::messages_first_chunk::frame_error::{frame_error_code, frame_error_object};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _code = frame_error_code(bytes);
    let _object = frame_error_object(bytes);
});
