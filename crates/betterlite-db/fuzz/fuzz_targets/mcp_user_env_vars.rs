#![no_main]

use std::str::from_utf8;

use betterlite_db::mcp_user_env_vars::codec::parse_user_env_vars;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(text) = from_utf8(bytes) {
        let _values = parse_user_env_vars(text);
    }
});
