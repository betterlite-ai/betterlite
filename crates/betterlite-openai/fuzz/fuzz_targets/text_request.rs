#![no_main]

use betterlite_openai::text_completion_request::CompletionRequest;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(request) = CompletionRequest::parse(bytes) {
        let _messages = request.prompt.messages();
        let _params = request.optional_params();
    }
});
