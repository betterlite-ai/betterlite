#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::azure_credentials::{DEFAULT_SCOPE, DeploymentSettings, Settings};

const BASE: &str = "model_list:\n  - model_name: azure-credentials\n    litellm_params:\n      model: azure/gpt-4o\n";

fn load_source(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.azure_credentials)
}

fn load(extra: &str, variables: &[(&str, &str)]) -> Settings {
    load_source(&format!("{BASE}{extra}"), variables).unwrap()
}

fn deployment(settings: &Settings) -> &DeploymentSettings {
    settings.deployments.first().unwrap()
}

// every credential key reads `explicit or environment`; a secret also stays out of Debug output and is redacted
fn check_field(
    key: &str,
    variable: &str,
    read: fn(&DeploymentSettings) -> Option<String>,
    secret: bool,
) {
    let explicit = format!("      {key}: fixture-explicit\n");
    let empty = format!("      {key}: \"\"\n");
    let environment = [(variable, "fixture-environment")];
    assert_eq!(read(deployment(&load("", &[]))), None);
    assert_eq!(
        read(deployment(&load(&explicit, &[]))).as_deref(),
        Some("fixture-explicit")
    );
    assert_eq!(
        read(deployment(&load("", &environment))).as_deref(),
        Some("fixture-environment")
    );
    assert_eq!(
        read(deployment(&load(&explicit, &environment))).as_deref(),
        Some("fixture-explicit")
    );
    assert_eq!(
        read(deployment(&load(&empty, &environment))).as_deref(),
        Some("fixture-environment")
    );
    if secret {
        let settings = load(&explicit, &[]);
        assert!(!format!("{settings:?}").contains("fixture-explicit"));
        assert_eq!(
            settings.redact_known("echo fixture-explicit back"),
            "echo [redacted] back"
        );
    }
}

#[rstest]
#[case::loads_azure_scope(0)]
#[case::loads_enable_azure_ad_token_refresh(1)]
#[case::loads_azure_tenant_id(2)]
#[case::loads_azure_client_id(3)]
#[case::loads_azure_client_secret(4)]
#[case::loads_azure_username(5)]
#[case::loads_azure_password(6)]
#[case::loads_azure_ad_token(7)]
#[case::rejects_unrelated_key(8)]
fn azure_credentials_config_cases(#[case] case: u8) {
    match case {
        0 => loads_azure_scope(),
        1 => loads_enable_azure_ad_token_refresh(),
        2 => check_field(
            "tenant_id",
            "AZURE_TENANT_ID",
            |settings| settings.tenant_id.clone(),
            false,
        ),
        3 => check_field(
            "client_id",
            "AZURE_CLIENT_ID",
            |settings| settings.client_id.clone(),
            false,
        ),
        4 => check_field(
            "client_secret",
            "AZURE_CLIENT_SECRET",
            |settings| {
                settings
                    .client_secret
                    .as_ref()
                    .map(|value| value.expose_secret().to_owned())
            },
            true,
        ),
        5 => check_field(
            "azure_username",
            "AZURE_USERNAME",
            |settings| settings.azure_username.clone(),
            false,
        ),
        6 => check_field(
            "azure_password",
            "AZURE_PASSWORD",
            |settings| {
                settings
                    .azure_password
                    .as_ref()
                    .map(|value| value.expose_secret().to_owned())
            },
            true,
        ),
        7 => check_field(
            "azure_ad_token",
            "AZURE_AD_TOKEN",
            |settings| {
                settings
                    .azure_ad_token
                    .as_ref()
                    .map(|value| value.expose_secret().to_owned())
            },
            true,
        ),
        _ => rejects_unrelated_key(),
    }
}

fn loads_azure_scope() {
    assert_eq!(deployment(&load("", &[])).azure_scope, DEFAULT_SCOPE);
    assert_eq!(
        deployment(&load("      azure_scope: explicit-scope\n", &[])).azure_scope,
        "explicit-scope"
    );
    assert_eq!(
        deployment(&load("", &[("AZURE_SCOPE", "environment-scope")])).azure_scope,
        "environment-scope"
    );
    assert_eq!(
        deployment(&load(
            "      azure_scope: explicit-scope\n",
            &[("AZURE_SCOPE", "environment-scope")],
        ))
        .azure_scope,
        "explicit-scope"
    );
    assert_eq!(
        deployment(&load("", &[("AZURE_SCOPE", "")])).azure_scope,
        ""
    );
    assert_eq!(
        deployment(&load(
            "      azure_scope: \"\"\n",
            &[("AZURE_SCOPE", "environment-scope")],
        ))
        .azure_scope,
        "environment-scope"
    );
}

fn loads_enable_azure_ad_token_refresh() {
    assert!(!load("", &[]).enable_azure_ad_token_refresh);
    let enabled = load_source(
        "litellm_settings:\n  enable_azure_ad_token_refresh: true\n",
        &[],
    )
    .unwrap();
    assert!(enabled.enable_azure_ad_token_refresh);
    let disabled = load_source(
        "litellm_settings:\n  enable_azure_ad_token_refresh: false\n",
        &[],
    )
    .unwrap();
    assert!(!disabled.enable_azure_ad_token_refresh);
}

fn rejects_unrelated_key() {
    let error = load_source(
        "litellm_settings:\n  azure_credentials_unrelated_key: 1\n",
        &[],
    )
    .unwrap_err();
    let ConfigError::Unsupported { paths } = error else {
        panic!("the unrelated key loaded without an unsupported-key error");
    };
    assert_eq!(
        paths,
        vec!["litellm_settings.azure_credentials_unrelated_key".to_owned()]
    );
}
