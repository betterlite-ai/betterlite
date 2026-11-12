#![no_main]

use betterlite_auth::auth_cache::decode::decode_cached_auth;
use betterlite_auth::auth_cache::payload::ReadOperation;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_cached_auth(bytes, ReadOperation::Get));
    drop(decode_cached_auth(bytes, ReadOperation::AsyncGet));
});
