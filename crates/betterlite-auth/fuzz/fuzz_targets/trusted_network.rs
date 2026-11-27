#![no_main]

use betterlite_auth::trusted_network::decode::decode_network_headers;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_network_headers(bytes));
});
