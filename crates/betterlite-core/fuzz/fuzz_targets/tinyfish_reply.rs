#![no_main]
use betterlite_core::api::search_providers::tinyfish::TINYFISH_RESULT_CAP;
use betterlite_core::api::search_providers::tinyfish::reply::decode_tinyfish_search;
use betterlite_openai::openai_base_config::Headers;
use libfuzzer_sys::fuzz_target;
use std::str;
fuzz_target!(|bytes: &[u8]| {
    if let Ok(text) = str::from_utf8(bytes) {
        let headers = Headers::new();
        if let Ok(response) = decode_tinyfish_search(text, 200, &headers, TINYFISH_RESULT_CAP) {
            drop(response.results);
        }
    }
});
