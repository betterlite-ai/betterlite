#![no_main]

use betterlite_openai::small_chat_hosts::nadir::nadir_reported_cost;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _cost = nadir_reported_cost(bytes);
});
