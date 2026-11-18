#![no_main]

use betterlite_models::decisions::Json;
use betterlite_server::member_request_contracts::decode_all;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(value) = serde_json::from_slice::<Json>(bytes) else {
        return;
    };
    let _accepted = decode_all(&value);
});
