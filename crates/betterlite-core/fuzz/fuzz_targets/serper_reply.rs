#![no_main]

use betterlite_core::api::search_providers::serper::reply::decode_serper_search;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(decode_serper_search(data));
});
