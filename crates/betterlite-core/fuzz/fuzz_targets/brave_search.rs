#![no_main]

use betterlite_core::api::search_providers::brave::dates::{DateFlags, DateValue, to_yyyy_mm_dd};
use betterlite_core::api::search_providers::brave::reply::decode_brave_search;
use chrono::NaiveDate;
use libfuzzer_sys::fuzz_target;

const URL: &str = "https://api.search.brave.com/res/v1/web/search?count=3&result_filter=web,news";

fuzz_target!(|data: &[u8]| {
    let today = NaiveDate::from_ymd_opt(2024, 3, 5).unwrap_or_default();
    drop(decode_brave_search(data, Some(URL), today));
    if let Ok(text) = std::str::from_utf8(data) {
        drop(to_yyyy_mm_dd(
            &DateValue::from_literal(text),
            today,
            DateFlags::default(),
        ));
    }
});
