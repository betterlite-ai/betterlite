use std::time::Duration;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::support::{environment, init_with_secret, required_environment};
use crate::azure_sentinel_logging::settings::{SentinelInit, SentinelSettings};

#[test]
fn resolves_every_setting_from_the_environment() {
    let settings = SentinelSettings::resolve(&SentinelInit::default(), &required_environment())
        .expect("the required environment resolves");
    assert_eq!(
        settings.model_url,
        "https://dce.example.test/dataCollectionRules/dcr-1/streams/Custom-LiteLLM?api-version=2023-01-01"
    );
    assert_eq!(settings.audit_url, settings.model_url);
    assert_eq!(
        settings.token_url,
        "https://login.microsoftonline.com/tenant-1/oauth2/v2.0/token"
    );
    assert_eq!(settings.scope, "https://monitor.azure.com/.default");
    assert_eq!(settings.client_id, "client-1");
    assert_eq!(settings.client_secret.expose_secret(), "secret-1");
    assert_eq!(settings.batch.batch_size, 512);
    assert_eq!(settings.batch.flush_interval, Duration::from_secs(5));
    assert_eq!(settings.batch.max_queue_size, 50_000);
}

#[test]
fn arguments_win_over_the_environment() {
    let init = SentinelInit {
        dcr_immutable_id: Some("dcr-arg".to_owned()),
        stream_name: Some("Custom-Arg".to_owned()),
        endpoint: Some("https://arg.example.test/".to_owned()),
        tenant_id: Some("tenant-arg".to_owned()),
        client_id: Some("client-arg".to_owned()),
        audit_stream_name: Some("Custom-Audit".to_owned()),
        authority_host: Some("login.microsoftonline.us/".to_owned()),
        ..init_with_secret("secret-arg")
    };
    let settings =
        SentinelSettings::resolve(&init, &required_environment()).expect("the arguments resolve");
    assert_eq!(
        settings.model_url,
        "https://arg.example.test/dataCollectionRules/dcr-arg/streams/Custom-Arg?api-version=2023-01-01"
    );
    assert_eq!(
        settings.audit_url,
        "https://arg.example.test/dataCollectionRules/dcr-arg/streams/Custom-Audit?api-version=2023-01-01"
    );
    assert_eq!(
        settings.token_url,
        "https://login.microsoftonline.us/tenant-arg/oauth2/v2.0/token"
    );
    assert_eq!(settings.client_id, "client-arg");
    assert_eq!(settings.client_secret.expose_secret(), "secret-arg");
}

#[test]
fn an_empty_argument_falls_through_to_the_environment() {
    let init = SentinelInit {
        dcr_immutable_id: Some(String::new()),
        ..init_with_secret("")
    };
    let settings = SentinelSettings::resolve(&init, &required_environment())
        .expect("empty arguments fall through");
    assert!(settings.model_url.contains("/dcr-1/"));
    assert_eq!(settings.client_secret.expose_secret(), "secret-1");
}

#[rstest]
#[case::dcr(
    "AZURE_SENTINEL_DCR_IMMUTABLE_ID",
    "AZURE_SENTINEL_DCR_IMMUTABLE_ID is required. Set it as an environment variable or pass dcr_immutable_id parameter."
)]
#[case::endpoint(
    "AZURE_SENTINEL_ENDPOINT",
    "AZURE_SENTINEL_ENDPOINT is required. Set it as an environment variable or pass endpoint parameter."
)]
#[case::tenant(
    "AZURE_SENTINEL_TENANT_ID",
    "AZURE_SENTINEL_TENANT_ID or AZURE_TENANT_ID is required. Set it as an environment variable or pass tenant_id parameter."
)]
#[case::client_id(
    "AZURE_SENTINEL_CLIENT_ID",
    "AZURE_SENTINEL_CLIENT_ID or AZURE_CLIENT_ID is required. Set it as an environment variable or pass client_id parameter."
)]
#[case::client_secret(
    "AZURE_SENTINEL_CLIENT_SECRET",
    "AZURE_SENTINEL_CLIENT_SECRET or AZURE_CLIENT_SECRET is required. Set it as an environment variable or pass client_secret parameter."
)]
fn a_missing_required_value_names_itself(#[case] missing: &str, #[case] message: &str) {
    let mut variables = required_environment();
    variables.remove(missing);
    let error = SentinelSettings::resolve(&SentinelInit::default(), &variables)
        .expect_err("a required value is missing");
    assert_eq!(error.to_string(), message);
}

#[rstest]
#[case::dcr(&[], "AZURE_SENTINEL_DCR_IMMUTABLE_ID")]
#[case::endpoint(&[("AZURE_SENTINEL_DCR_IMMUTABLE_ID", "dcr-1")], "AZURE_SENTINEL_ENDPOINT")]
#[case::tenant(
    &[
        ("AZURE_SENTINEL_DCR_IMMUTABLE_ID", "dcr-1"),
        ("AZURE_SENTINEL_ENDPOINT", "https://dce.example.test"),
        ("AZURE_SENTINEL_CLIENT_SECRET", "secret-1"),
    ],
    "AZURE_SENTINEL_TENANT_ID"
)]
fn the_first_missing_value_in_litellm_order_is_reported(
    #[case] given: &[(&str, &str)],
    #[case] first_missing: &str,
) {
    let error = SentinelSettings::resolve(&SentinelInit::default(), &environment(given))
        .expect_err("a value is missing");
    assert!(error.to_string().starts_with(first_missing));
}

#[test]
fn batch_variables_size_the_queues() {
    let mut variables = required_environment();
    variables.insert("DEFAULT_BATCH_SIZE".to_owned(), "7".to_owned());
    variables.insert("DEFAULT_FLUSH_INTERVAL_SECONDS".to_owned(), "9".to_owned());
    let settings = SentinelSettings::resolve(&SentinelInit::default(), &variables)
        .expect("the batch variables resolve");
    assert_eq!(settings.batch.batch_size, 7);
    assert_eq!(settings.batch.flush_interval, Duration::from_secs(9));
}

#[test]
fn an_unreadable_batch_variable_is_an_error() {
    let mut variables = required_environment();
    variables.insert("DEFAULT_BATCH_SIZE".to_owned(), "many".to_owned());
    SentinelSettings::resolve(&SentinelInit::default(), &variables)
        .expect_err("the batch size is not a number");
}
