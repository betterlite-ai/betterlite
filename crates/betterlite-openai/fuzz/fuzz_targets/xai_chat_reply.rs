#![no_main]

use betterlite_openai::xai_chat::response::{ReplyInput, transform_response};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _converted = transform_response(&ReplyInput { body: bytes });
});
