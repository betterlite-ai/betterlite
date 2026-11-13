#![no_main]

use betterlite_logging::zerobus::client::token_response::decode_token_response;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Err(failure) = decode_token_response(bytes, 0.0) {
        assert!(!failure.retryable);
    }
});
