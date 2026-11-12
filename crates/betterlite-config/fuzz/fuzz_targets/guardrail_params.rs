#![no_main]
use betterlite_config::sections::guardrail_params::decode::{
    decode_apply_request, decode_litellm_params, decode_patch_request,
};
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let _params = decode_litellm_params(bytes);
    let _patch = decode_patch_request(bytes);
    let _apply = decode_apply_request(bytes);
});
