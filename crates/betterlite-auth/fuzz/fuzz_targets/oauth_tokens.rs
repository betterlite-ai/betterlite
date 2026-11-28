#![no_main]

use betterlite_auth::oauth_tokens::decode_oauth_response;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_oauth_response(bytes));
});
