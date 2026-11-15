#![no_main]

use betterlite_core::api::search_providers::nimble::reply::decode_nimble_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(decode_nimble_reply(data));
});
