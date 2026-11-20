#![no_main]

use betterlite_core::api::fine_tuning_read::page::decode_page;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(decode_page(data));
});
