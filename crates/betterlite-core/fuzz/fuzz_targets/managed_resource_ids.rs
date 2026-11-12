#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::managed_resource_ids::decode_managed_resource_ids;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_managed_resource_ids(bytes, ApiLimits::default()));
});
