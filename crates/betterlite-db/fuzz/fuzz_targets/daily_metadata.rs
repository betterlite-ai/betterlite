#![no_main]

use betterlite_db::daily_records::metadata::{decode_daily_metadata, request_status};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _metadata = decode_daily_metadata(bytes);
    let _status = request_status(Some(bytes));
});
