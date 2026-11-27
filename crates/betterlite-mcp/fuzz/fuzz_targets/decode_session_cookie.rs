#![no_main]

use betterlite_mcp::session_admission::decode_session_cookie::decode_session_cookie;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_session_cookie(bytes);
});
