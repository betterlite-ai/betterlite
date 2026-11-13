#![no_main]

use betterlite_openai::aleph_alpha::reply::decode_aleph_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_aleph_reply(200, bytes);
});
