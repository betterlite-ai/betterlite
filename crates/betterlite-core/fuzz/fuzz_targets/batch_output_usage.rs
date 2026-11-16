#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::batch_output_usage::decode_batch_output_usage;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_batch_output_usage(bytes, &ApiLimits::default()));
});
