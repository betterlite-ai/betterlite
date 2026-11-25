#![no_main]

use betterlite_server::mcp::outbound_credentials::sso_assertion_refresher::transport::map_answer;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| drop(map_answer(200, bytes)));
