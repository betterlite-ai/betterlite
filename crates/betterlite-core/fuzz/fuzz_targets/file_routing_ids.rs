#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::file_routing_ids::decode_file_routing_ids;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_file_routing_ids(bytes, ApiLimits::default()));
});
