#![no_main]

use std::str;

use betterlite_redis::redis_cache_admin::info_parse::{parse_client_list, parse_info};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = str::from_utf8(data) else {
        return;
    };
    drop(parse_info(text));
    drop(parse_client_list(text));
});
