#![no_main]

use betterlite_openai::chat_choices::decode_chat_choices;
use libfuzzer_sys::fuzz_target;
use uuid::Uuid;

fuzz_target!(|bytes: &[u8]| {
    let _choices = decode_chat_choices(bytes, None, false, Uuid::nil);
});
