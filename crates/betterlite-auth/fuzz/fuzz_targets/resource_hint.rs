#![no_main]

use betterlite_auth::resource_hints::{ResourceKind, decode_resource_hint};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_resource_hint(ResourceKind::Video, bytes));
    drop(decode_resource_hint(ResourceKind::Character, bytes));
});
