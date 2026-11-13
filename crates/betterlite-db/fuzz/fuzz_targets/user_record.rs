#![no_main]

use betterlite_db::user_repository::projection::decode_user_record;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_user_record(bytes);
});
