#![no_main]

use betterlite_openai::chat::ChatRequest;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _request = ChatRequest::parse(bytes);
});
