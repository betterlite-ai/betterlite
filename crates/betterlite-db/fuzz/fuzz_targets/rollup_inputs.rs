#![no_main]

use betterlite_db::model_usage::decode::decode_model_usage_fields;
use betterlite_db::window_queue::stamp::decode_window_stamp;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Some((&tag, rest)) = bytes.split_first() else {
        return;
    };
    if (tag & 1) == 0 {
        drop(decode_window_stamp(rest));
    } else {
        let mut parts = rest.split(|byte| *byte == 0);
        let metadata = parts.next().unwrap_or_default();
        let request_tags = parts.next().unwrap_or_default();
        drop(decode_model_usage_fields(metadata, request_tags));
    }
});
