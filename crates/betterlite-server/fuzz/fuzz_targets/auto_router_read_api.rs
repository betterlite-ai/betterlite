#![no_main]

use betterlite_server::auto_router_read_api::query::{
    decode_benchmarks_query, decode_session_query,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _benchmarks = decode_benchmarks_query(bytes);
    let _session = decode_session_query(bytes);
});
