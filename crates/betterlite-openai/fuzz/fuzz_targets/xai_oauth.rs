#![no_main]

use betterlite_openai::xai_tokens::codec::decode_tagged;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _document = decode_tagged(bytes);
});
