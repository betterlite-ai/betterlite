#![no_main]

use betterlite_logging::otel::callbacks::agentops::parse_token_response;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    let _auth = parse_token_response(body);
});
