#![no_main]

use betterlite_server::mcp::outbound_credentials::client_credentials::token_post::{
    TokenEndpointOutcome, parse_token_response,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    match parse_token_response(bytes, 200) {
        TokenEndpointOutcome::Success { body } => {
            let first = bytes.iter().find(|byte| !byte.is_ascii_whitespace());
            assert_eq!(first, Some(&b'{'), "a success starts a JSON object");
            let _text = body.access_token_text();
        }
        TokenEndpointOutcome::Denied {
            status_code,
            detail,
        } => {
            assert_eq!(status_code, 200);
            assert_eq!(detail, "token endpoint returned a non-JSON-object body");
        }
        TokenEndpointOutcome::Unreachable { .. } => {}
    }
});
