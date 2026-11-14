#![no_main]

use betterlite_core::api::search_providers::parallel_ai::reply::decode_parallel_ai_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(decode_parallel_ai_reply(data));
});
