#![no_main]

use betterlite_auth::jwt_keys::{decode_discovery_document, userinfo_document};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_discovery_document(bytes));
    drop(userinfo_document(bytes));
});
