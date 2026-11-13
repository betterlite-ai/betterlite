#![no_main]

use betterlite_openai::copilot::device_auth::decode::{decode_access_reply, decode_device_reply};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _device = decode_device_reply(bytes);
    let _access = decode_access_reply(bytes);
});
