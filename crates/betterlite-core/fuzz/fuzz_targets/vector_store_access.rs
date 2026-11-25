#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::vector_store_access::decode::decode_vector_store_access;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_vector_store_access(bytes, ApiLimits::default()));
});
