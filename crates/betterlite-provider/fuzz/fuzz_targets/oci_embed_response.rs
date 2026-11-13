#![no_main]

use betterlite_provider::oci_embed::response::decode_embed_response;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _accepted = decode_embed_response(200, bytes);
    let _rejected = decode_embed_response(500, bytes);
});
