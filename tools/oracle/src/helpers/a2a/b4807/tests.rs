#![cfg(test)]

use std::path::Path;

use rstest::rstest;

use crate::case::{LoadedCase, load_cases};
use crate::helpers::{HelperRun, run_case};

#[rstest]
#[case::config_extract_cp4d("config-extract-cp4d")]
#[case::config_extract_ibm_cloud("config-extract-ibm-cloud")]
#[case::config_missing_required("config-missing-required")]
#[case::config_cp4d_missing_username("config-cp4d-missing-username")]
#[case::token_cp4d_basic("token-cp4d-basic")]
#[case::token_ibm_cloud_basic("token-ibm-cloud-basic")]
#[case::token_cache_hit("token-cache-hit")]
#[case::token_cp4d_expiry("token-cp4d-expiry")]
#[case::token_ibm_cloud_expiry("token-ibm-cloud-expiry")]
#[case::text_extract_a2a_params_single("text-extract-a2a-params-single")]
#[case::text_extract_a2a_params_multiple("text-extract-a2a-params-multiple")]
#[case::text_extract_a2a_params_no_text("text-extract-a2a-params-no-text")]
#[case::result_extract_last_message("result-extract-last-message")]
#[case::result_extract_nested_data("result-extract-nested-data")]
#[case::result_extract_raw_string("result-extract-raw-string")]
#[case::result_extract_empty("result-extract-empty")]
#[case::response_build_a2a_message("response-build-a2a-message")]
#[case::fake_streaming_basic("fake-streaming-basic")]
#[case::fake_streaming_empty_text("fake-streaming-empty-text")]
#[case::fake_streaming_large_text("fake-streaming-large-text")]
#[case::fake_streaming_delays("fake-streaming-delays")]
#[case::fake_streaming_multibyte_text("fake-streaming-multibyte-text")]
fn watsonx_orchestrate(#[case] name: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("cases/watsonx-orchestrate/{name}.yaml"));
    let cases = load_cases(&[path]).unwrap();
    let LoadedCase::Helper(case) = &cases[0] else {
        panic!("expected helper");
    };
    assert!(matches!(run_case(case, false).unwrap(), HelperRun::Match));
}
