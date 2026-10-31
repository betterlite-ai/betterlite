#![no_main]

use betterlite_db::encryption_scan::decode_migration_json;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = decode_migration_json(bytes);
});
