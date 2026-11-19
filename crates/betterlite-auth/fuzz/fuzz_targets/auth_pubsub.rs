#![no_main]

use betterlite_auth::auth_pubsub::decode::decode_invalidation;
use betterlite_auth::auth_pubsub::message::read;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(typed) = decode_invalidation(bytes) {
        drop(read(&typed));
    }
});
