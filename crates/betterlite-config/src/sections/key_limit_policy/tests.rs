#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use rust_decimal::Decimal;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::key_limit_policy::{Settings, UpperboundKeyGenerateParams};

const KEY: &str = "litellm_settings.upperbound_key_generate_params";

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .key_limit_policy)
}

fn amount(text: &str) -> Decimal {
    text.parse().unwrap()
}

#[test]
fn loads_settings() {
    let source = "\
litellm_settings:
  upperbound_key_generate_params:
    max_budget: os.environ/KEY_BUDGET
    budget_duration: os.environ/KEY_BUDGET_DURATION
    duration: 30d
    max_parallel_requests: 8
    tpm_limit: os.environ/KEY_TPM
    rpm_limit: 60";
    let loaded = settings(
        source,
        &[
            ("KEY_BUDGET", "250.50"),
            ("KEY_BUDGET_DURATION", "1mo"),
            ("KEY_TPM", "100000"),
        ],
    )
    .expect("every upperbound key loads");
    assert_eq!(
        loaded.upperbound_key_generate_params,
        Some(UpperboundKeyGenerateParams {
            max_budget: Some(amount("250.50")),
            budget_duration: Some("1mo".to_owned()),
            duration: Some("30d".to_owned()),
            max_parallel_requests: Some(8),
            tpm_limit: Some(100_000),
            rpm_limit: Some(60),
        })
    );
}

#[test]
fn upperbound_absent() {
    let absent = settings("litellm_settings: {}", &[]).expect("an empty section loads");
    assert_eq!(absent.upperbound_key_generate_params, None);
    let null = settings(
        "litellm_settings:\n  upperbound_key_generate_params: null",
        &[],
    )
    .expect("an explicit null loads");
    assert_eq!(null.upperbound_key_generate_params, None);
}

#[test]
fn upperbound_numeric_and_duration() {
    let source = "\
litellm_settings:
  upperbound_key_generate_params:
    max_budget: 0
    budget_duration: \"-1\"
    duration: daily
    max_parallel_requests: -2
    tpm_limit: 0
    rpm_limit: 0.0";
    let loaded = settings(source, &[]).expect("zero and negative ceilings load");
    assert_eq!(
        loaded.upperbound_key_generate_params,
        Some(UpperboundKeyGenerateParams {
            max_budget: Some(Decimal::ZERO),
            budget_duration: Some("-1".to_owned()),
            duration: Some("daily".to_owned()),
            max_parallel_requests: Some(-2),
            tpm_limit: Some(0),
            rpm_limit: Some(0),
        })
    );
}

#[rstest]
#[case::boolean_budget("max_budget: true", "max_budget")]
#[case::boolean_parallel("max_parallel_requests: true", "max_parallel_requests")]
#[case::text_tpm("tpm_limit: fast", "tpm_limit")]
#[case::number_duration("duration: 30", "duration")]
fn invalid_upperbound_type(#[case] entry: &str, #[case] field: &str) {
    let source = format!("litellm_settings:\n  upperbound_key_generate_params:\n    {entry}");
    let error = settings(&source, &[]).expect_err("a wrong type fails the load");
    assert!(
        error.to_string().contains(&format!("{KEY}.{field}")),
        "{entry}: {error}"
    );
}

// LiteLLM keeps the float and hands out keys whose ceiling never binds
#[rstest]
#[case::nan(".nan")]
#[case::inf(".inf")]
fn non_finite_budget_fails_the_load(#[case] value: &str) {
    let source =
        format!("litellm_settings:\n  upperbound_key_generate_params:\n    max_budget: {value}");
    let error = settings(&source, &[]).expect_err("a non-finite budget fails the load");
    assert!(
        matches!(error.category(), "yaml" | "invalid_scalar"),
        "{value}: {error}"
    );
}

// a finite budget past rust_decimal's range, such as 1e29, fails where LiteLLM keeps the float
#[test]
fn decimal_out_of_range_budget_fails_the_load() {
    let source = "litellm_settings:\n  upperbound_key_generate_params:\n    max_budget: 1.0e29";
    let error =
        settings(source, &[]).expect_err("a budget past rust_decimal's range fails the load");
    assert_eq!(error.category(), "invalid_scalar");
    assert!(error.to_string().contains(KEY), "{error}");
}

#[test]
fn missing_env_reference() {
    let missing = settings(
        "litellm_settings:\n  upperbound_key_generate_params:\n    rpm_limit: os.environ/MISSING",
        &[],
    )
    .expect_err("a missing environment reference fails the load");
    assert!(missing.to_string().contains("MISSING"), "{missing}");
    assert!(missing.to_string().contains(KEY), "{missing}");
}
