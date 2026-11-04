#![no_main]

use betterlite_db::daily_spend_sql::value_decode::decode_daily_value;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _outcome = decode_daily_value(bytes);
});
