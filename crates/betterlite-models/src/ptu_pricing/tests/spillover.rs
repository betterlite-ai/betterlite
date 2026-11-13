use std::collections::BTreeMap;

use rstest::rstest;

use super::model_info;
use crate::ptu_pricing::PtuModelInfo;
use crate::ptu_pricing::spillover::{AzureSpillover, azure_spillover, is_spilled_over_ptu_request};
use crate::tiered_pricing::Scalar;

fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn spillover(
    response_pairs: Option<&[(&str, &str)]>,
    additional_pairs: Option<&[(&str, &str)]>,
) -> Option<AzureSpillover> {
    let response = response_pairs.map(headers);
    let additional = additional_pairs.map(headers);
    azure_spillover(response.as_ref(), additional.as_ref())
}

#[test]
fn spillover_prefers_the_unprefixed_response_headers() {
    assert_eq!(
        spillover(
            Some(&[
                ("x-ms-is-spilled-over", "true"),
                ("x-ms-spillover-from-deployment", "dep-response"),
            ]),
            Some(&[
                ("llm_provider-x-ms-is-spilled-over", "true"),
                (
                    "llm_provider-x-ms-spillover-from-deployment",
                    "dep-additional"
                ),
            ]),
        ),
        Some(AzureSpillover {
            from_deployment: Some("dep-response".to_owned()),
        })
    );
}

#[test]
fn spillover_reads_the_prefixed_additional_headers_when_the_response_has_none() {
    assert_eq!(
        spillover(
            Some(&[("x-ms-is-spilled-over", "false")]),
            Some(&[
                ("llm_provider-x-ms-is-spilled-over", "true"),
                (
                    "llm_provider-x-ms-spillover-from-deployment",
                    "dep-additional"
                ),
            ]),
        ),
        Some(AzureSpillover {
            from_deployment: Some("dep-additional".to_owned()),
        })
    );
}

#[test]
fn spillover_ignores_an_unprefixed_additional_header() {
    assert_eq!(
        spillover(
            None,
            Some(&[
                ("x-ms-is-spilled-over", "true"),
                ("x-ms-spillover-from-deployment", "dep-additional"),
            ]),
        ),
        None
    );
}

#[rstest]
#[case::upper_case("TRUE")]
#[case::mixed_case("True")]
fn spillover_compares_the_value_case_insensitively(#[case] value: &str) {
    assert_eq!(
        spillover(Some(&[("x-ms-is-spilled-over", value)]), None),
        Some(AzureSpillover {
            from_deployment: None,
        })
    );
}

#[rstest]
#[case::padded(" true ")]
#[case::numeric("1")]
#[case::boolean_word("yes")]
#[case::empty("")]
fn spillover_rejects_any_other_value(#[case] value: &str) {
    assert_eq!(
        spillover(Some(&[("x-ms-is-spilled-over", value)]), None),
        None
    );
}

#[test]
fn spillover_reports_a_missing_from_deployment() {
    assert_eq!(
        spillover(Some(&[("x-ms-is-spilled-over", "true")]), None),
        Some(AzureSpillover {
            from_deployment: None,
        })
    );
}

#[test]
fn spillover_keeps_the_from_deployment_empty_when_it_is_absent() {
    assert_eq!(
        spillover(
            Some(&[("x-ms-is-spilled-over", "true")]),
            Some(&[(
                "llm_provider-x-ms-spillover-from-deployment",
                "dep-additional",
            )]),
        ),
        Some(AzureSpillover {
            from_deployment: None,
        })
    );
}

#[test]
fn spillover_returns_none_without_headers() {
    assert_eq!(spillover(None, None), None);
}

fn valid_model_info() -> BTreeMap<String, Scalar> {
    model_info(
        r#"{"ptu_count":1,"cost_per_ptu_per_hour":1,"team_id":"team-1","ptu_effective_from":"2026-01-01T00:00:00Z"}"#,
    )
}

#[test]
fn spilled_over_requires_valid_terms_enabled_attribution_and_the_header() {
    let info = valid_model_info();
    let model = PtuModelInfo::from(&info);
    let spilled = headers(&[("x-ms-is-spilled-over", "true")]);
    assert!(is_spilled_over_ptu_request(
        &model,
        Some(&spilled),
        None,
        true
    ));
    assert!(!is_spilled_over_ptu_request(
        &model,
        Some(&spilled),
        None,
        false
    ));
    assert!(!is_spilled_over_ptu_request(&model, None, None, true));
}

#[test]
fn spilled_over_is_false_for_unchargeable_terms_even_when_enabled() {
    let info = model_info(r#"{"ptu_count":1}"#);
    let model = PtuModelInfo::from(&info);
    let spilled = headers(&[("x-ms-is-spilled-over", "true")]);
    assert!(!is_spilled_over_ptu_request(
        &model,
        Some(&spilled),
        None,
        true
    ));
}
