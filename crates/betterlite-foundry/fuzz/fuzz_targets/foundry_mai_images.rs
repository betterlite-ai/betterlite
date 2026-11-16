#![no_main]

use betterlite_foundry::mai_images::reply::decode_mai_image_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_mai_image_reply(bytes, 200);
});
