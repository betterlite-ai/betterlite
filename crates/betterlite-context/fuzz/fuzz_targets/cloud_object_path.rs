#![no_main]

use betterlite_context::cloud_storage::decode_cloud_object_reference;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    std::str::from_utf8(input)
        .map(decode_cloud_object_reference)
        .ok();
});
