#![no_main]
use betterlite_context::cloud_storage::decode_cloud_object_reference;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|value: &str| {
    let _ = decode_cloud_object_reference(value, "gs://", "bucket", &[], true);
});
