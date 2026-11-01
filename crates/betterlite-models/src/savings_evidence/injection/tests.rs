#![cfg(test)]

use rstest::rstest;

use super::marks_gateway_injection;
use crate::savings_evidence::metadata::Metadata;
use crate::tiered_pricing::Scalar;

fn metadata_with_marker(marker: Scalar) -> Metadata {
    Metadata {
        litellm_gateway_injected_cache: Some(marker),
        compression_savings: None,
        guardrail_information: None,
        autorouter_savings_estimate: None,
        autorouter_savings: None,
    }
}

#[rstest]
#[case::empty_marker_matches_every_deployment(
    Scalar::Text(String::new()),
    Some("deployment-a"),
    true
)]
#[case::empty_marker_matches_a_missing_model_id(Scalar::Text(String::new()), None, true)]
#[case::exact_model_id_match(Scalar::Text("deployment-a".to_owned()), Some("deployment-a"), true)]
#[case::sibling_leg_does_not_match(Scalar::Text("deployment-a".to_owned()), Some("deployment-b"), false)]
#[case::nonstring_marker_never_attributes(
    Scalar::Number(serde_json::Number::from(5_i32)),
    Some("deployment-a"),
    false
)]
fn marks_gateway_injection_matches_the_exact_deployment_or_every_deployment(
    #[case] marker: Scalar,
    #[case] model_id: Option<&str>,
    #[case] expected: bool,
) {
    let metadata = metadata_with_marker(marker);
    assert_eq!(marks_gateway_injection(Some(&metadata), model_id), expected);
}

#[test]
fn marks_gateway_injection_is_false_without_metadata() {
    assert!(!marks_gateway_injection(None, Some("deployment-a")));
}

#[test]
fn marks_gateway_injection_is_false_without_the_marker_key() {
    assert!(!marks_gateway_injection(
        Some(&Metadata::default()),
        Some("deployment-a")
    ));
}
