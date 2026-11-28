#![no_main]

use betterlite_auth::auth_context_sources::router_settings::decode_router_settings;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_router_settings(bytes));
});
