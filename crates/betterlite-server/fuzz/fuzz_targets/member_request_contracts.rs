#![no_main]

use betterlite_server::member_request_contracts::decode_all;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(value) = serde_json::from_slice(bytes) else {
        return;
    };
    decode_all(&value);
});
