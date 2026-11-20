#![no_main]

use betterlite_db::spend_rows::key::{WindowDurationValidity, decode_spend_counter_key};
use libfuzzer_sys::fuzz_target;

struct AnyDuration;

impl WindowDurationValidity for AnyDuration {
    fn is_valid(&self, _spec: &str) -> bool {
        true
    }
}

struct NoDuration;

impl WindowDurationValidity for NoDuration {
    fn is_valid(&self, _spec: &str) -> bool {
        false
    }
}

fuzz_target!(|bytes: &[u8]| {
    let Some((&tag, rest)) = bytes.split_first() else {
        return;
    };
    if (tag & 1) == 0 {
        let _decoded = decode_spend_counter_key(rest, &AnyDuration);
    } else {
        let _decoded = decode_spend_counter_key(rest, &NoDuration);
    }
});
