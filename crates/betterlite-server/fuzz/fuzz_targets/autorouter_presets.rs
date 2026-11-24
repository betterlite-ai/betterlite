#![no_main]

use betterlite_server::public_endpoints::router_presets::catalog::decode_catalog;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _catalog = decode_catalog(data);
});
