#![no_main]

use betterlite_foundry::image_edits::reply::decode_mai_edit_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_mai_edit_reply(bytes, 200, 1_700_000_000);
});
