#![no_main]

use betterlite_core::api::search_providers::dataforseo::reply::decode_dataforseo_search;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(decode_dataforseo_search(data));
});
