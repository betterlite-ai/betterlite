#![no_main]

use betterlite_azure::image_transport::decode::decode_image_operation;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _operation = decode_image_operation(bytes);
});
