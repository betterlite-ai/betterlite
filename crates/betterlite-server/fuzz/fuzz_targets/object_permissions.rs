#![no_main]

use betterlite_server::object_permissions::decode_object_permission;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_object_permission(bytes));
});
