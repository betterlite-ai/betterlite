#![no_main]

use betterlite_core::api::anthropic_beta_config::matrix::{
    decode_beta_matrix, validate_beta_matrix,
};
use betterlite_core::api::api_boundaries::ApiLimits;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(node) = decode_beta_matrix(data, ApiLimits::default()) {
        drop(validate_beta_matrix(node));
    }
});
