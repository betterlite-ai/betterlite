#![no_main]

use betterlite_server::mcp_user_env_routes::body::decode_user_env_vars_request;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_user_env_vars_request(None, bytes));
});
