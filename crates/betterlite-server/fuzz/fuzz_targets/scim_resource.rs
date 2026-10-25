#![no_main]
use betterlite_server::scim_records::{ResourceKind, decode_scim_resource};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let kind = match bytes.first() {
        Some(b'u') => ResourceKind::User,
        Some(b'g') => ResourceKind::Group,
        _ => ResourceKind::Patch,
    };
    let body = bytes.get(1..).unwrap_or_default();
    drop(decode_scim_resource(body, kind));
});
