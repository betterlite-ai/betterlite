#![no_main]
use betterlite_server::benchmark_aggregates::decode::decode_session_agg_rows;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _rows = decode_session_agg_rows(bytes);
});
