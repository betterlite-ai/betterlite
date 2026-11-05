#![no_main]

use betterlite_openai::assistants::decode::decode_assistant_object;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _assistant = decode_assistant_object(bytes);
});
