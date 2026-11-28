#![no_main]
use betterlite_server::config_update::body::decode_config_update;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_config_update(bytes);
});
