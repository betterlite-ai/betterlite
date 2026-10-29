#![no_main]

use betterlite_db::database_url_policy::decoder::decode_database_url;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _decoded = decode_database_url(bytes);
});
