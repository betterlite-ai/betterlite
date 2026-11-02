#![no_main]

use betterlite_db::daily_spend_sql::tags::decode_daily_tags;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = decode_daily_tags(bytes);
});
