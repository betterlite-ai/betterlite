#![no_main]

use betterlite_openai::copilot::api_key::decode::decode_copilot_key_record;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _record = decode_copilot_key_record(bytes);
});
