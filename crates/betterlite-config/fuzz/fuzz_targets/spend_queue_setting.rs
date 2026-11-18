#![no_main]
use betterlite_config::spend_queue::{
    AGGREGATION_THRESHOLD_ENV, DEFAULT_LOCK_TTL_ENV, FLUSH_COUNT_ENV, QUEUE_CAPACITY_ENV,
    decode_spend_queue_setting,
};
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    for name in [
        QUEUE_CAPACITY_ENV,
        AGGREGATION_THRESHOLD_ENV,
        FLUSH_COUNT_ENV,
        DEFAULT_LOCK_TTL_ENV,
    ] {
        let _decoded = decode_spend_queue_setting(name, text);
    }
});
