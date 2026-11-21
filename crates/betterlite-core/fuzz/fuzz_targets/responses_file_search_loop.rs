#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_file_search_loop::decode_responses_file_search_args;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_responses_file_search_args(
        bytes,
        &ApiLimits::default(),
    ));
});
