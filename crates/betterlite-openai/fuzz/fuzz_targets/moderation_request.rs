#![no_main]

use betterlite_openai::moderation_request::ModerationRequest;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(request) = ModerationRequest::parse(bytes) {
        let _body = request.upstream_body("omni-moderation-latest");
    }
});
