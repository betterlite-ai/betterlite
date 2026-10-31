#![no_main]

use betterlite_db::resource_rows::decode_resource_rows;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = decode_resource_rows(bytes);
});
