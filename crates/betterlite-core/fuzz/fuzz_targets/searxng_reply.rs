#![no_main]

use betterlite_core::api::search_providers::searxng::decode_searxng_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(decode_searxng_reply(data));
});
