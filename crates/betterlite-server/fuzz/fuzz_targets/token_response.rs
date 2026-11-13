#![no_main]

use betterlite_server::mcp::outbound_credentials::client_credentials::token_post::{
    TokenEndpointOutcome, parse_token_response,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    match parse_token_response(bytes, 200) {
        TokenEndpointOutcome::Success { body } => {
            let object =
                std::str::from_utf8(bytes).is_ok_and(|text| text.trim_start().starts_with('{'));
            assert!(object, "a success implies a JSON object came in");
            let _text = body.access_token_text();
        }
        TokenEndpointOutcome::Denied {
            status_code,
            detail,
        } => {
            assert_eq!(status_code, 200);
            assert_eq!(detail, "token endpoint returned a non-JSON-object body");
        }
        TokenEndpointOutcome::Unreachable { .. } => {
            panic!("a decode never reports an unreachable endpoint");
        }
    }
});
