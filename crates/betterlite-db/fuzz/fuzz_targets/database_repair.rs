#![no_main]

use betterlite_db::database_repair::decode_verification_csv;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_verification_csv(bytes);
});
