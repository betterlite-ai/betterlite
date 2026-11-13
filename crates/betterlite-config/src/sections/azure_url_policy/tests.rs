#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::azure_url_policy::{AzureCredentials, Settings};

const BASE: &str =
    "model_list:\n  - model_name: azure-urls\n    litellm_params:\n      model: azure/gpt-4o\n";

fn load_source(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.azure_url_policy)
}

fn load(extra: &str, variables: &[(&str, &str)]) -> Settings {
    load_source(&format!("{BASE}{extra}"), variables).unwrap()
}

fn deployment(settings: &Settings) -> &AzureCredentials {
    settings.deployments.first().unwrap()
}

fn api_key(settings: &Settings) -> Option<String> {
    deployment(settings)
        .api_key
        .as_ref()
        .map(|value| value.expose_secret().to_owned())
}

fn check_text_chain(key: &str, variable: &str, read: fn(&AzureCredentials) -> Option<String>) {
    assert_eq!(read(deployment(&load("", &[]))), None);
    let explicit = format!("      {key}: deployment-value\n");
    assert_eq!(
        read(deployment(&load(
            &explicit,
            &[(variable, "environment-value")]
        )))
        .as_deref(),
        Some("deployment-value")
    );
    let module = format!("litellm_settings:\n  {key}: module-value\n");
    assert_eq!(
        read(deployment(&load(
            &module,
            &[(variable, "environment-value")]
        )))
        .as_deref(),
        Some("module-value")
    );
    assert_eq!(
        read(deployment(&load(&module, &[]))).as_deref(),
        Some("module-value")
    );
    let blank = format!("      {key}: \"\"\n");
    assert_eq!(read(deployment(&load(&blank, &[]))), None);
    assert_eq!(
        read(deployment(&load("", &[(variable, "environment-value")]))).as_deref(),
        Some("environment-value")
    );
    assert_eq!(
        read(deployment(&load("", &[(variable, "")]))).as_deref(),
        Some("")
    );
    check_text_reference(key, variable, read);
}

fn check_text_reference(key: &str, variable: &str, read: fn(&AzureCredentials) -> Option<String>) {
    let reference = format!("      {key}: os.environ/{variable}\n");
    assert_eq!(
        read(deployment(&load(
            &reference,
            &[(variable, "environment-value")]
        )))
        .as_deref(),
        Some("environment-value")
    );
}

#[rstest]
#[case::loads_azure_api_base(0)]
#[case::loads_azure_openai_api_key(1)]
#[case::loads_azure_api_key(2)]
#[case::loads_azure_api_version(3)]
#[case::rejects_unrelated_key(4)]
fn azure_urls_config_cases(#[case] case: u8) {
    match case {
        0 => loads_azure_api_base(),
        1 => loads_azure_openai_api_key(),
        2 => loads_azure_api_key(),
        3 => loads_azure_api_version(),
        _ => rejects_unrelated_key(),
    }
}

fn loads_azure_api_base() {
    check_text_chain("api_base", "AZURE_API_BASE", |credentials| {
        credentials.api_base.clone()
    });
}

fn loads_azure_openai_api_key() {
    assert_eq!(api_key(&load("", &[])), None);
    assert_eq!(
        api_key(&load("", &[("AZURE_OPENAI_API_KEY", "environment-key")])).as_deref(),
        Some("environment-key")
    );
    assert_eq!(
        api_key(&load(
            "      api_key: deployment-key\n",
            &[("AZURE_OPENAI_API_KEY", "environment-key")],
        ))
        .as_deref(),
        Some("deployment-key")
    );
    assert_eq!(
        api_key(&load(
            "litellm_settings:\n  api_key: module-key\n",
            &[("AZURE_OPENAI_API_KEY", "environment-key")],
        ))
        .as_deref(),
        Some("module-key")
    );
    assert_eq!(
        api_key(&load(
            "litellm_settings:\n  azure_key: azure-key\n",
            &[("AZURE_OPENAI_API_KEY", "environment-key")],
        ))
        .as_deref(),
        Some("azure-key")
    );
    let settings = load("      api_key: fixture-secret\n", &[]);
    assert!(!format!("{settings:?}").contains("fixture-secret"));
    assert_eq!(
        settings.redact_known("echo fixture-secret back"),
        "echo [redacted] back"
    );
}

fn loads_azure_api_key() {
    assert_eq!(
        api_key(&load("", &[("AZURE_API_KEY", "last-key")])).as_deref(),
        Some("last-key")
    );
    assert_eq!(
        api_key(&load(
            "",
            &[
                ("AZURE_OPENAI_API_KEY", "openai-key"),
                ("AZURE_API_KEY", "last-key"),
            ],
        ))
        .as_deref(),
        Some("openai-key")
    );
    assert_eq!(
        api_key(&load(
            "",
            &[("AZURE_OPENAI_API_KEY", ""), ("AZURE_API_KEY", "last-key")],
        ))
        .as_deref(),
        Some("last-key")
    );
    assert_eq!(
        api_key(&load(
            "      api_key: os.environ/AZURE_API_KEY\n",
            &[("AZURE_API_KEY", "last-key")],
        ))
        .as_deref(),
        Some("last-key")
    );
}

fn loads_azure_api_version() {
    check_text_chain("api_version", "AZURE_API_VERSION", |credentials| {
        credentials.api_version.clone()
    });
}

fn rejects_unrelated_key() {
    let error = load_source("litellm_settings:\n  azure_urls_unrelated_key: 1\n", &[]).unwrap_err();
    let ConfigError::Unsupported { paths } = error else {
        panic!("the unrelated key loaded without an unsupported-key error");
    };
    assert_eq!(
        paths,
        vec!["litellm_settings.azure_urls_unrelated_key".to_owned()]
    );
}
