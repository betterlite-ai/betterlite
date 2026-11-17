#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_sse_recovery::{decode_recovery_line, recover_recorded_output};
use libfuzzer_sys::fuzz_target;

// the first byte tags the entry point: an even byte feeds the single-line decoder, an odd one the whole recording
fuzz_target!(|bytes: &[u8]| {
    let limits = ApiLimits::default();
    let Some((&tag, payload)) = bytes.split_first() else {
        return;
    };
    if tag.is_multiple_of(2) {
        drop(decode_recovery_line(payload, &limits));
    } else {
        drop(recover_recorded_output(payload, &limits));
    }
});
