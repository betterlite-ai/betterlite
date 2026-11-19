#![no_main]

use betterlite_config::sections::settings::JsonValue;
use betterlite_core::api::search_providers::duckduckgo::reply::decode_duckduckgo_search;
use betterlite_core::api::search_providers::duckduckgo::request::DuckDuckGoParams;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let params = DuckDuckGoParams {
        query: String::new(),
        format: String::new(),
        max_results: Some(JsonValue::from(2_i64)),
        pretty: None,
        no_redirect: None,
        no_html: None,
        skip_disambig: None,
    };
    drop(decode_duckduckgo_search(data, &params));
});
