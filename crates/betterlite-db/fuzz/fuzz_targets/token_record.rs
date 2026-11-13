#![no_main]

use betterlite_db::key_repository::decode::decode_token_record;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_token_record(bytes);
});
