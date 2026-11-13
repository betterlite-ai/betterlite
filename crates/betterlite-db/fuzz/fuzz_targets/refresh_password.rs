#![no_main]

use std::str;

use betterlite_db::database_refresh_loop::extraction::extract_refresh_password;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = str::from_utf8(bytes) else {
        return;
    };
    let _password = extract_refresh_password(text);
});
