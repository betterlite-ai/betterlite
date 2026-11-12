#![no_main]

use betterlite_db::database_refresh_loop::extraction::extract_refresh_password;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(text) = std::str::from_utf8(bytes) {
        let _password = extract_refresh_password(text);
    }
});
