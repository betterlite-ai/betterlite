use rstest::rstest;
use secrecy::SecretString;

use super::support::{
    DATASET_BODY, Reply, ScriptedRandom, ScriptedTransport, environment, logger, settings,
    transformation_json,
};
use crate::argilla_logging::logger::ArgillaLogger;
use crate::argilla_logging::settings::{
    ArgillaSettings, ConstructorArguments, SettingsError, digit_sampling_rate,
};
use crate::argilla_logging::transport::ArgillaMethod;
use std::time::Duration;

#[tokio::test]
async fn resolves_the_dataset_and_reads_every_default() {
    let built = logger(&[], &[Reply::ok(DATASET_BODY)], ScriptedRandom::new(&[])).await;
    assert_eq!(built.credentials.dataset_name, "ds-1");
    assert_eq!(built.credentials.base_url, "http://localhost:6900/");
    assert_eq!(built.sampling_rate.to_bits(), 1.0_f64.to_bits());
    assert_eq!(built.queue.settings.batch_size, 512);
    assert_eq!(built.queue.settings.flush_interval, Duration::from_secs(5));
    let calls = built.transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, ArgillaMethod::Get);
    assert_eq!(
        calls[0].url,
        "http://localhost:6900//api/v1/me/datasets?name=litellm-completion"
    );
    assert_eq!(calls[0].api_key, "argilla-key");
}

#[tokio::test]
async fn explicit_arguments_beat_the_environment() {
    let arguments = ConstructorArguments {
        api_key: Some(SecretString::from("argument-key")),
        dataset_name: Some("argument-name".to_owned()),
        base_url: Some("https://argument.example".to_owned()),
    };
    let pairs = environment(&[
        ("ARGILLA_API_KEY", "environment-key"),
        ("ARGILLA_DATASET_NAME", "environment-name"),
        ("ARGILLA_BASE_URL", "https://environment.example"),
    ]);
    let built = ArgillaSettings::from_environment(&pairs, &arguments, Some(&transformation_json()))
        .expect("the settings are valid");
    assert_eq!(built.credentials.base_url, "https://argument.example");
    assert_eq!(built.credentials.dataset_name, "argument-name");
    let transport = ScriptedTransport::answering(&[Reply::ok(DATASET_BODY)]);
    let connected = ArgillaLogger::connect(built, transport, ScriptedRandom::new(&[]))
        .await
        .expect("the logger connects");
    let calls = connected.transport.calls();
    assert_eq!(
        calls[0].url,
        "https://argument.example/api/v1/me/datasets?name=argument-name"
    );
    assert_eq!(calls[0].api_key, "argument-key");
    assert_eq!(connected.credentials.dataset_name, "ds-1");
}

#[test]
fn refuses_to_initialize_without_an_api_key() {
    let result = ArgillaSettings::from_environment(
        &environment(&[]),
        &ConstructorArguments::default(),
        Some(&transformation_json()),
    );
    assert!(matches!(result, Err(SettingsError::MissingApiKey)));
}

#[rstest]
#[case::digits("3", 3.0_f64)]
#[case::zero("0", 0.0_f64)]
#[case::padded(" 7 ", 7.0_f64)]
#[case::decimal("0.5", 1.0_f64)]
#[case::negative("-1", 1.0_f64)]
#[case::empty("", 1.0_f64)]
#[case::word("fast", 1.0_f64)]
fn samples_only_after_a_digit_only_rate(#[case] text: &str, #[case] expected: f64) {
    assert_eq!(
        digit_sampling_rate(Some(text)).to_bits(),
        expected.to_bits()
    );
}

#[rstest]
#[case::unset(&[], 1.0_f64)]
#[case::digits(&[("ARGILLA_SAMPLING_RATE", "5")], 5.0_f64)]
#[case::decimal(&[("ARGILLA_SAMPLING_RATE", "0.5")], 1.0_f64)]
fn reads_the_sampling_rate_from_the_environment(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: f64,
) {
    assert_eq!(settings(pairs).sampling_rate.to_bits(), expected.to_bits());
}

#[rstest]
#[case::default(&[], 512)]
#[case::base_variable(&[("DEFAULT_BATCH_SIZE", "7")], 7)]
#[case::argilla_variable(&[("ARGILLA_BATCH_SIZE", "2")], 2)]
#[case::argilla_wins(&[("DEFAULT_BATCH_SIZE", "7"), ("ARGILLA_BATCH_SIZE", "2")], 2)]
#[case::empty_argilla_falls_back(&[("DEFAULT_BATCH_SIZE", "7"), ("ARGILLA_BATCH_SIZE", "")], 7)]
fn resolves_the_batch_size(#[case] pairs: &[(&str, &str)], #[case] expected: usize) {
    assert_eq!(settings(pairs).batch.batch_size, expected);
}

#[test]
fn rejects_a_non_numeric_batch_size() {
    let result = ArgillaSettings::from_environment(
        &environment(&[("ARGILLA_API_KEY", "key"), ("ARGILLA_BATCH_SIZE", "many")]),
        &ConstructorArguments::default(),
        Some(&transformation_json()),
    );
    assert!(matches!(result, Err(SettingsError::BatchSize(_))));
}

#[test]
fn keeps_the_b256_flush_interval() {
    let built = settings(&[("DEFAULT_FLUSH_INTERVAL_SECONDS", "9")]);
    assert_eq!(built.batch.flush_interval, Duration::from_secs(9));
}
