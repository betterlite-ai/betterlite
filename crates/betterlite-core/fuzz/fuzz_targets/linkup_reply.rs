#![no_main]

use betterlite_core::api::search_providers::linkup::decode_linkup_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(decode_linkup_reply(data));
});
