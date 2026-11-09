#![no_main]

use betterlite_core::api::search_providers::firecrawl::reply::decode_firecrawl_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(response) = decode_firecrawl_reply(data) {
        drop(response.results);
    }
});
