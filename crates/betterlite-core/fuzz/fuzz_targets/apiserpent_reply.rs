#![no_main]

use betterlite_core::api::search_providers::apiserpent::reply::decode_apiserpent_search;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(decode_apiserpent_search(data));
});
