#![no_main]

use betterlite_prometheus::multiprocess::files::decode_db_file;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _entries = decode_db_file(bytes);
});
