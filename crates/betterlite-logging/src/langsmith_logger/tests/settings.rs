use std::time::Duration;

use rstest::rstest;
use secrecy::ExposeSecret;

use super::support::environment;
use crate::langsmith_logger::settings::{
    BATCH_SIZE_VARIABLE, DEFAULT_RUN_NAME_VARIABLE, SAMPLING_RATE_VARIABLE, resolve,
};

#[rstest]
#[case::unset(&[], 1.0_f64)]
#[case::one(&[(SAMPLING_RATE_VARIABLE, "1")], 1.0_f64)]
#[case::ten(&[(SAMPLING_RATE_VARIABLE, "10")], 10.0_f64)]
#[case::fraction_falls_back(&[(SAMPLING_RATE_VARIABLE, "0.5")], 1.0_f64)]
#[case::zero(&[(SAMPLING_RATE_VARIABLE, "0")], 0.0_f64)]
#[case::padded(&[(SAMPLING_RATE_VARIABLE, " 2 ")], 2.0_f64)]
#[case::empty_falls_back(&[(SAMPLING_RATE_VARIABLE, "")], 1.0_f64)]
#[case::negative_falls_back(&[(SAMPLING_RATE_VARIABLE, "-1")], 1.0_f64)]
fn sampling_rate_tests_behavior(#[case] environment_pairs: &[(&str, &str)], #[case] expected: f64) {
    let settings = resolve(&environment(environment_pairs), None, false)
        .expect("the LangSmith settings resolve");
    assert_eq!(settings.sampling_rate.to_bits(), expected.to_bits());
}

#[rstest]
#[case::unset(&[], "LLMRun")]
#[case::configured(&[(DEFAULT_RUN_NAME_VARIABLE, "my-run")], "my-run")]
#[case::empty_falls_back(&[(DEFAULT_RUN_NAME_VARIABLE, "")], "LLMRun")]
fn default_run_name_tests_behavior(
    #[case] environment_pairs: &[(&str, &str)],
    #[case] expected: &str,
) {
    let settings = resolve(&environment(environment_pairs), None, false)
        .expect("the LangSmith settings resolve");
    assert_eq!(settings.default_run_name, expected);
}

#[rstest]
#[case::environment(&[(BATCH_SIZE_VARIABLE, "64")], Some(32), 64)]
#[case::configured(&[], Some(32), 32)]
#[case::unspecified(&[], None, 512)]
#[case::shared_environment(&[("DEFAULT_BATCH_SIZE", "600")], None, 600)]
#[case::empty_environment(&[(BATCH_SIZE_VARIABLE, "")], Some(32), 32)]
#[case::explicit_zero(&[(BATCH_SIZE_VARIABLE, "0")], None, 0)]
#[case::negative_environment(&[(BATCH_SIZE_VARIABLE, "-1")], None, 0)]
#[case::negative_configured(&[], Some(-5), 0)]
fn batch_size_tests_behavior(
    #[case] environment_pairs: &[(&str, &str)],
    #[case] configured: Option<i64>,
    #[case] expected: usize,
) {
    let settings = resolve(&environment(environment_pairs), configured, false)
        .expect("the LangSmith settings resolve");
    assert_eq!(settings.batch.batch_size, expected);
}

#[test]
fn a_non_numeric_batch_size_names_the_variable() {
    let environment = environment(&[(BATCH_SIZE_VARIABLE, "many")]);
    let error = resolve(&environment, None, false).expect_err("the batch size is not a number");
    assert!(error.to_string().contains(BATCH_SIZE_VARIABLE));
}

#[rstest]
#[case::defaults(&[], 5)]
#[case::shared_environment(&[("DEFAULT_FLUSH_INTERVAL_SECONDS", "9")], 9)]
fn flush_interval_tests_behavior(
    #[case] environment_pairs: &[(&str, &str)],
    #[case] expected: u64,
) {
    let settings = resolve(&environment(environment_pairs), None, false)
        .expect("the LangSmith settings resolve");
    assert_eq!(settings.batch.flush_interval, Duration::from_secs(expected));
}

#[test]
fn default_credentials_come_from_the_environment() {
    let environment = environment(&[
        ("LANGSMITH_API_KEY", "key-from-env"),
        ("LANGSMITH_PROJECT", "project-from-env"),
        ("LANGSMITH_BASE_URL", "https://env.example"),
        ("LANGSMITH_TENANT_ID", "tenant-from-env"),
    ]);
    let settings = resolve(&environment, None, false).expect("the LangSmith settings resolve");
    let credentials = settings.default_credentials;
    assert_eq!(
        credentials
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("key-from-env")
    );
    assert_eq!(credentials.project, "project-from-env");
    assert_eq!(credentials.base_url, "https://env.example");
    assert_eq!(credentials.tenant_id.as_deref(), Some("tenant-from-env"));
}

#[test]
fn the_redaction_flag_comes_from_the_configuration() {
    let environment = environment(&[]);
    let settings = resolve(&environment, None, true).expect("the LangSmith settings resolve");
    assert!(settings.redact_user_api_key_info);
}
