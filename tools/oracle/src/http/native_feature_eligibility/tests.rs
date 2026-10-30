#![cfg(test)]

use std::collections::BTreeMap;
use std::path::Path;

use crate::case::{Case, decode_http_case, validate};

fn case(extra: &str) -> String {
    let body = "name: scim\nconfig: |\n  general_settings: {}\n";
    format!(
        "{body}env: {{{extra}}}\nrequest: {{method: GET, path: /scim/v2/ServiceProviderConfig, headers: {{}}, body: null}}\nupstream: []\ncompare: {{headers: [], headers_present: [], upstream_headers: [], ignore: []}}\n"
    )
}

#[test]
fn absent_eligibility_is_stock_behavior() {
    let decoded = decode_http_case(case("").as_bytes()).expect("case decodes");
    assert!(!decoded.native_feature_eligibility);
}

#[test]
fn explicit_eligibility_decodes() {
    let decoded =
        decode_http_case(format!("native_feature_eligibility: true\n{}", case("")).as_bytes())
            .expect("case decodes");
    assert!(decoded.native_feature_eligibility);
}

#[rstest::rstest]
#[case("LITELLM_LICENSE")]
#[case("LITELLM_WORKER_STARTUP_HOOKS")]
fn controlled_environment_values_are_rejected(#[case] key: &str) {
    let result = decode_http_case(case(&format!("\n  {key}: value")).as_bytes());
    let _error = result.unwrap_err();
}

#[test]
fn controlled_config_license_is_rejected() {
    let source = case("").replace(
        "general_settings: {}",
        "general_settings: {litellm_license: value}",
    );
    let result = decode_http_case(source.as_bytes());
    let _error = result.unwrap_err();
}

#[test]
fn validation_accepts_unrelated_environment_and_config() {
    let source = case("\n  LITELLM_LOCAL_MODEL_COST_MAP: 'True'");
    let parsed: Case = serde_saphyr::from_str(&source).expect("case decodes");
    validate(Path::new("<test>"), &parsed).expect("unrelated settings are allowed");
    assert_eq!(
        parsed.env,
        BTreeMap::from([(
            String::from("LITELLM_LOCAL_MODEL_COST_MAP"),
            String::from("True")
        )])
    );
}
