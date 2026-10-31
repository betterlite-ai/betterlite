#![no_main]

use betterlite_db::identity_rows::decode_identity_rows;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = decode_identity_rows(bytes);
});
