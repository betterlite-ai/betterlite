#![no_main]
use betterlite_server::scim_records::{ResourceKind, decode_scim_resource};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    for kind in [ResourceKind::User, ResourceKind::Group, ResourceKind::Patch] {
        let _ = decode_scim_resource(bytes, kind);
    }
});
