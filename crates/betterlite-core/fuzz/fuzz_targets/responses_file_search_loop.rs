#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
// ast-grep-ignore: no-long-names
use betterlite_core::api::responses_file_search_loop::decode_responses_file_search_args;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    // ast-grep-ignore: no-long-names
    drop(decode_responses_file_search_args(
        bytes,
        &ApiLimits::default(),
    ));
});
