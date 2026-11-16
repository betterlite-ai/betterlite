#![cfg(test)]

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, run_case};
use rstest::rstest;
use std::fs;
use std::path::Path;

#[rstest]
#[case::providers_list("public-endpoints/providers-list")]
#[case::providers_fields_assembled("public-endpoints/providers-fields-assembled")]
#[case::endpoints_discovery("public-endpoints/endpoints-discovery")]
#[case::clean_display_name("helpers/public-endpoints/clean-display-name")]
#[case::unknown_endpoint_label_path("helpers/public-endpoints/unknown-endpoint-label-path")]
#[case::endpoint_insertion_order("helpers/public-endpoints/endpoint-insertion-order")]
#[case::fragment_source_order("helpers/public-endpoints/fragment-source-order")]
#[case::cache_reuse("helpers/public-endpoints/cache-reuse")]
fn replays_b3190_against_litellm(#[case] name: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let source = fs::read_to_string(root.join(format!("cases/{name}.yaml"))).unwrap();
    let case = serde_saphyr::from_str::<HelperCase>(&source).unwrap();
    let result = run_case(&[crate::GROUP], &case, false).unwrap();
    assert!(matches!(result, HelperRun::Match));
}
