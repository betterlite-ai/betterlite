#![no_main]

use betterlite_server::spend::totals::query::{LimitQuery, decode_spend_totals_query};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let decoded = decode_spend_totals_query(bytes);
    if let Some(api_key) = decoded.api_key {
        let encoded: String = form_urlencoded::byte_serialize(api_key.as_bytes()).collect();
        let again = decode_spend_totals_query(format!("api_key={encoded}").as_bytes());
        assert_eq!(again.api_key, Some(api_key));
    }
    if let LimitQuery::Value(limit) = decoded.limit {
        let again = decode_spend_totals_query(format!("limit={limit}").as_bytes());
        assert_eq!(again.limit, decoded.limit);
    }
});
