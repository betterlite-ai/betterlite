#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::batch_input_accounting::decode_batch_input_accounting;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_batch_input_accounting(bytes, &ApiLimits::default()));
});
