#![no_main]

use betterlite_server::spend::global_report::query::decode_global_report_query;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let decoded = decode_global_report_query(bytes);
    drop(decoded);
});
