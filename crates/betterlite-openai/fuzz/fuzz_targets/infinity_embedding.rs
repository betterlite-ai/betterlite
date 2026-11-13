#![no_main]

use betterlite_openai::infinity_embedding::decode_infinity_embedding;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _reply = decode_infinity_embedding(bytes, 502);
});
