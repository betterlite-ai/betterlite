#![no_main]

use betterlite_server::list_api::plan::query::decode_list_query;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let decoded = decode_list_query(bytes);
    assert_eq!(
        decoded.pairs.len(),
        decoded.last_value.len().max(1).min(decoded.pairs.len())
    );
});
