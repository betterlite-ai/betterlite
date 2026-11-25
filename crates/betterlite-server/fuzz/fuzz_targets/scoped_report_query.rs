#![no_main]

use betterlite_server::spend::reports::query::decode_scoped_report_query;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let decoded = decode_scoped_report_query(bytes);
    drop(decoded);
});
