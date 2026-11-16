#![no_main]

use betterlite_db::resource_repository::config::decode_config_json;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_config_json(bytes);
});
