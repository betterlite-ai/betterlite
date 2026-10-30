#![no_main]

use betterlite_db::team_rows::decode_team_rows;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = decode_team_rows(bytes);
});
