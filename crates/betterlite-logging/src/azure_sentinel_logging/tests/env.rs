use secrecy::ExposeSecret as _;

use super::support::{Environment, environment, required_environment};
use crate::azure_sentinel_logging::settings::{SentinelInit, SentinelSettings};

fn resolve(variables: &Environment) -> SentinelSettings {
    SentinelSettings::resolve(&SentinelInit::default(), variables)
        .expect("the environment resolves")
}

fn with(pairs: &[(&str, &str)]) -> Environment {
    let mut variables = required_environment();
    variables.extend(environment(pairs));
    variables
}

#[test]
fn the_stream_name_defaults_to_custom_litellm() {
    let settings = resolve(&required_environment());
    assert!(settings.model_url.contains("/streams/Custom-LiteLLM?"));
}

#[test]
fn the_stream_name_variable_names_the_model_stream() {
    let settings = resolve(&with(&[("AZURE_SENTINEL_STREAM_NAME", "Custom-Models")]));
    assert!(settings.model_url.contains("/streams/Custom-Models?"));
    assert!(settings.audit_url.contains("/streams/Custom-Models?"));
}

#[test]
fn the_audit_stream_defaults_to_the_model_stream_and_can_differ() {
    let settings = resolve(&with(&[
        ("AZURE_SENTINEL_STREAM_NAME", "Custom-Models"),
        ("AZURE_SENTINEL_AUDIT_STREAM_NAME", "Custom-Audit"),
    ]));
    assert!(settings.model_url.contains("/streams/Custom-Models?"));
    assert!(settings.audit_url.contains("/streams/Custom-Audit?"));
}

#[test]
fn empty_variables_count_as_unset() {
    let settings = resolve(&with(&[
        ("AZURE_SENTINEL_STREAM_NAME", ""),
        ("AZURE_SENTINEL_AUDIT_STREAM_NAME", ""),
        ("AZURE_SENTINEL_AUTHORITY_HOST", ""),
    ]));
    assert!(settings.model_url.contains("/streams/Custom-LiteLLM?"));
    assert!(settings.audit_url.contains("/streams/Custom-LiteLLM?"));
    assert!(
        settings
            .token_url
            .starts_with("https://login.microsoftonline.com/")
    );
}

#[test]
fn the_generic_azure_variables_back_up_the_sentinel_ones() {
    let mut variables = required_environment();
    variables.remove("AZURE_SENTINEL_TENANT_ID");
    variables.remove("AZURE_SENTINEL_CLIENT_ID");
    variables.remove("AZURE_SENTINEL_CLIENT_SECRET");
    variables.extend(environment(&[
        ("AZURE_TENANT_ID", "tenant-generic"),
        ("AZURE_CLIENT_ID", "client-generic"),
        ("AZURE_CLIENT_SECRET", "secret-generic"),
        ("AZURE_AUTHORITY_HOST", "login.microsoftonline.us"),
    ]));
    let settings = resolve(&variables);
    assert_eq!(
        settings.token_url,
        "https://login.microsoftonline.us/tenant-generic/oauth2/v2.0/token"
    );
    assert_eq!(settings.client_id, "client-generic");
    assert_eq!(settings.client_secret.expose_secret(), "secret-generic");
}

#[test]
fn the_sentinel_variables_win_over_the_generic_ones() {
    let settings = resolve(&with(&[
        ("AZURE_TENANT_ID", "tenant-generic"),
        ("AZURE_CLIENT_ID", "client-generic"),
        ("AZURE_CLIENT_SECRET", "secret-generic"),
        ("AZURE_AUTHORITY_HOST", "login.microsoftonline.us"),
        ("AZURE_SENTINEL_AUTHORITY_HOST", "login.example.test"),
    ]));
    assert_eq!(
        settings.token_url,
        "https://login.example.test/tenant-1/oauth2/v2.0/token"
    );
    assert_eq!(settings.client_id, "client-1");
    assert_eq!(settings.client_secret.expose_secret(), "secret-1");
}

#[test]
fn an_empty_sentinel_variable_falls_back_to_the_generic_one() {
    let settings = resolve(&with(&[
        ("AZURE_SENTINEL_TENANT_ID", ""),
        ("AZURE_TENANT_ID", "tenant-generic"),
    ]));
    assert!(settings.token_url.contains("/tenant-generic/"));
}

#[test]
fn the_endpoint_loses_its_trailing_slashes() {
    let settings = resolve(&with(&[(
        "AZURE_SENTINEL_ENDPOINT",
        "https://dce.example.test//",
    )]));
    assert!(
        settings
            .model_url
            .starts_with("https://dce.example.test/dataCollectionRules/dcr-1/")
    );
}
