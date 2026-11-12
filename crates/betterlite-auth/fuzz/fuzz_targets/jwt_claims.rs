#![no_main]

use betterlite_auth::jwt_claims::decode::decode_unverified_claims;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_unverified_claims(bytes));
});
