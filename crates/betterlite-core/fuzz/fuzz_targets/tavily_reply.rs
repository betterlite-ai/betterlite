#![no_main]

use betterlite_core::api::search_providers::tavily::reply::decode_tavily_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(decode_tavily_reply(data));
});
