#![no_main]

use betterlite_cli::pkce_login::request::parse_request;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = parse_request(bytes);
});
