#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_codex_tools::custom_tools::unwrap_custom_arguments;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(unwrap_custom_arguments(data, ApiLimits::default()));
    let small = ApiLimits {
        max_json_bytes: 64,
        ..ApiLimits::default()
    };
    drop(unwrap_custom_arguments(data, small));
});
