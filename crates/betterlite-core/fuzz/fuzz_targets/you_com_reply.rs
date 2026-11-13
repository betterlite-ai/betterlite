#![no_main]

use betterlite_core::api::search_providers::you_com::reply::decode_you_com_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(decode_you_com_reply(data));
});
