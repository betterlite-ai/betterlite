#![no_main]

use betterlite_server::team_admin_fields::decoder::decode_team_admin_object;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(text) = std::str::from_utf8(bytes) {
        drop(decode_team_admin_object(text));
    }
});
