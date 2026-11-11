#![no_main]

use betterlite_openai::volcengine_embedding::reply::decode_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _reply = decode_reply(bytes, "fuzz-model");
});
