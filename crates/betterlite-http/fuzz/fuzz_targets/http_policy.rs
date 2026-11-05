#![no_main]

use betterlite_http::tls::parse_ca_pem;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _result = parse_ca_pem(bytes);
});
