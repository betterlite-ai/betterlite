#![no_main]

use betterlite_openai::xai_login::callback::decode_xai_callback;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _request = decode_xai_callback(bytes);
});
