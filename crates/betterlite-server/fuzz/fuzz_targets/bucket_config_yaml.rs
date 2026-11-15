#![no_main]

use betterlite_server::bucket_config::object::parse_bucket_object;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _value = parse_bucket_object("config.yaml", bytes);
});
