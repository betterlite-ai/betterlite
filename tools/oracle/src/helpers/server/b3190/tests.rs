#![cfg(test)]

use rstest::rstest;

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
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(root.join(format!("cases/{name}.yaml"))).unwrap();
    let case = serde_saphyr::from_str::<crate::case::HelperCase>(&source).unwrap();
    let result = crate::helpers::run_case(&case, false).unwrap();
    assert!(matches!(result, crate::helpers::HelperRun::Match));
}
