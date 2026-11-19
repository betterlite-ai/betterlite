#![no_main]

use std::str::from_utf8;

use betterlite_server::team_admin_fields::decoder::decode_team_admin_object;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = from_utf8(bytes) else {
        return;
    };
    drop(decode_team_admin_object(text));
});
