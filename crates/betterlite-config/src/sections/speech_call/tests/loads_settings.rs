#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::loader::ConfigLoader;
use crate::sections::speech_call::Settings;

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.speech_call)
        .map_err(|error| error.to_string())
}

#[test]
fn reads_configured_values() {
    let source = "litellm_settings:\n  api_base: https://global.example\n  api_key: global-key\n  openai_key: openai-key\n  organization: org-global\n  project: project-global\n  azure_key: azure-key\n  api_version: 2024-02-01\n  headers: {x-global: 'yes'}\n  num_retries: 5\n  request_timeout: 12.5\n";
    let settings = settings(source, &[]).expect("settings load");
    assert_eq!(settings.api_base.as_deref(), Some("https://global.example"));
    assert_eq!(
        settings.api_key.expect("api key").expose_secret(),
        "global-key"
    );
    assert_eq!(
        settings.openai_key.expect("OpenAI key").expose_secret(),
        "openai-key"
    );
    assert_eq!(settings.organization.as_deref(), Some("org-global"));
    assert_eq!(settings.project.as_deref(), Some("project-global"));
    assert_eq!(
        settings.azure_key.expect("Azure key").expose_secret(),
        "azure-key"
    );
    assert_eq!(settings.api_version.as_deref(), Some("2024-02-01"));
    let headers = settings.headers.expect("headers");
    assert_eq!(headers.get("x-global").map(String::as_str), Some("yes"));
    assert_eq!(settings.num_retries, Some(5));
    assert_eq!(
        settings.request_timeout_seconds.to_bits(),
        12.5_f64.to_bits()
    );
}

#[test]
fn defaults_an_empty_config() {
    let settings = settings("litellm_settings: {}\n", &[]).expect("settings load");
    assert_eq!(
        settings.request_timeout_seconds.to_bits(),
        6_000.0_f64.to_bits()
    );
    assert_eq!(settings.num_retries, None);
    assert!(settings.api_base.is_none());
    assert!(settings.api_key.is_none());
    assert!(settings.openai_key.is_none());
    assert!(settings.azure_key.is_none());
    assert!(settings.api_version.is_none());
    assert!(settings.headers.is_none());
}

#[test]
fn reads_openai_environment_values() {
    let variables = [
        ("OPENAI_BASE_URL", "https://env.example/v1"),
        ("OPENAI_API_BASE", "https://legacy.example/v1"),
        ("OPENAI_API_KEY", "env-openai-key"),
        ("OPENAI_ORGANIZATION", "org-env"),
        ("OPENAI_PROJECT", "project-env"),
    ];
    let settings = settings("litellm_settings: {}\n", &variables).expect("settings load");
    assert_eq!(
        settings.openai_base_url.as_deref(),
        Some("https://env.example/v1")
    );
    assert_eq!(
        settings.openai_api_base.as_deref(),
        Some("https://legacy.example/v1")
    );
    assert_eq!(
        settings
            .openai_api_key
            .expect("environment OpenAI key")
            .expose_secret(),
        "env-openai-key"
    );
    assert_eq!(settings.openai_organization.as_deref(), Some("org-env"));
    assert_eq!(settings.openai_project.as_deref(), Some("project-env"));
}

#[test]
fn reads_azure_environment_values() {
    let variables = [
        ("AZURE_API_BASE", "https://azure.example"),
        ("AZURE_API_VERSION", "2024-06-01"),
        ("AZURE_OPENAI_API_KEY", "env-azure-openai-key"),
        ("AZURE_API_KEY", "env-azure-key"),
        ("AZURE_AD_TOKEN", "env-ad-token"),
    ];
    let settings = settings("litellm_settings: {}\n", &variables).expect("settings load");
    assert_eq!(
        settings.azure_api_base.as_deref(),
        Some("https://azure.example")
    );
    assert_eq!(settings.azure_api_version.as_deref(), Some("2024-06-01"));
    assert_eq!(
        settings
            .azure_openai_api_key
            .expect("environment Azure OpenAI key")
            .expose_secret(),
        "env-azure-openai-key"
    );
    assert_eq!(
        settings
            .azure_api_key
            .expect("environment Azure key")
            .expose_secret(),
        "env-azure-key"
    );
    assert_eq!(
        settings
            .azure_ad_token
            .expect("environment AD token")
            .expose_secret(),
        "env-ad-token"
    );
}

#[rstest]
#[case::unset_defaults(6_000.0_f64, &[])]
#[case::environment_value(7.0_f64, &[("REQUEST_TIMEOUT", "7")])]
#[case::fractional_environment_value(0.5_f64, &[("REQUEST_TIMEOUT", "0.5")])]
#[case::zero_is_kept(0.0_f64, &[("REQUEST_TIMEOUT", "0")])]
#[case::empty_environment_is_unset(6_000.0_f64, &[("REQUEST_TIMEOUT", "")])]
fn resolves_the_request_timeout(#[case] expected: f64, #[case] variables: &[(&str, &str)]) {
    let settings = settings("litellm_settings: {}\n", variables).expect("settings load");
    assert_eq!(
        settings.request_timeout_seconds.to_bits(),
        expected.to_bits()
    );
}

#[test]
fn resolves_environment_references() {
    let source =
        "litellm_settings:\n  api_base: os.environ/SPEECH_BASE\n  api_key: os.environ/SPEECH_KEY\n";
    let settings = settings(
        source,
        &[
            ("SPEECH_BASE", "https://ref.example"),
            ("SPEECH_KEY", "ref-key"),
        ],
    )
    .expect("settings load");
    assert_eq!(settings.api_base.as_deref(), Some("https://ref.example"));
    assert_eq!(
        settings.api_key.expect("api key").expose_secret(),
        "ref-key"
    );
}

#[test]
fn rejects_an_invalid_retry_count() {
    let error = settings("litellm_settings: {num_retries: soon}\n", &[]).unwrap_err();
    assert!(error.contains("litellm_settings.num_retries"), "{error}");
}

#[test]
fn reads_the_shipped_seed() {
    let source = include_str!("../../../../fuzz/corpus/config_yaml/speech-call-valid.yaml");
    let settings = settings(source, &[]).expect("seed loads");
    assert_eq!(
        settings.api_base.as_deref(),
        Some("https://seed.example/v1")
    );
    assert_eq!(settings.num_retries, Some(3));
}
