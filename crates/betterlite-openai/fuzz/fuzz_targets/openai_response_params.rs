#![no_main]

// ast-grep-ignore: no-long-names
use betterlite_openai::response_params::decode::decode_response_capability_error as decode_body;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _body = decode_body(bytes);
});
