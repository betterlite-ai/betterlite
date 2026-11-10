#![no_main]
use betterlite_config::spend_storage::decode_spend_storage_policy;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let _policy = decode_spend_storage_policy(bytes);
});
