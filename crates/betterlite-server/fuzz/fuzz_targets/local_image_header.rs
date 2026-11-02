#![no_main]
use betterlite_server::assets::images::detect_local_image_media_type;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _media_type = detect_local_image_media_type(bytes);
});
