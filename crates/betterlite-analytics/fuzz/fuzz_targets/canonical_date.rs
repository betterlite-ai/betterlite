#![no_main]

use std::str;

use betterlite_analytics::canonical_date::{parse_canonical_date, parse_canonical_date_range};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = str::from_utf8(bytes) else {
        return;
    };
    let _date = parse_canonical_date(text);
    let _range = parse_canonical_date_range(Some(text), Some(text));
});
