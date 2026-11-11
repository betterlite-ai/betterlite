#![no_main]

use betterlite_azure::azure_certificate::material::decode_certificate_material;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _material = decode_certificate_material(bytes, None);
});
