#![no_main]

use betterlite_db::baseline::records::decode::decode_baseline_json;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_baseline_json(bytes);
});
