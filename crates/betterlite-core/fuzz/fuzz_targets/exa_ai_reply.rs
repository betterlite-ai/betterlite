#![no_main]

use betterlite_core::api::search_providers::exa_ai::reply::decode_exa_ai_search;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(decode_exa_ai_search(data));
});
