#![no_main]

use std::str::from_utf8;

use betterlite_secret_managers::aws_secret_reads::parse_primary_secret;
use libfuzzer_sys::fuzz_target;
use secrecy::SecretString;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(text) = from_utf8(bytes) {
        let _parsed = parse_primary_secret(Some(&SecretString::from(text)));
    }
});
