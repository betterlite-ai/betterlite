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

fn text(settings: &Settings, read: fn(&DeploymentSettings) -> Option<String>) -> Option<String> {
    read(deployment(settings))
}

fn secret(
    settings: &Settings,
    read: fn(&DeploymentSettings) -> Option<&secrecy::SecretString>,
) -> Option<String> {
    read(deployment(settings)).map(|value| value.expose_secret().to_owned())
}

fn tenant(settings: &DeploymentSettings) -> Option<String> {
    settings.tenant_id.clone()
}

fn client(settings: &DeploymentSettings) -> Option<String> {
    settings.client_id.clone()
}

fn username(settings: &DeploymentSettings) -> Option<String> {
    settings.azure_username.clone()
}

fn client_secret(settings: &DeploymentSettings) -> Option<&secrecy::SecretString> {
    settings.client_secret.as_ref()
}

fn password(settings: &DeploymentSettings) -> Option<&secrecy::SecretString> {
    settings.azure_password.as_ref()
}

fn ad_token(settings: &DeploymentSettings) -> Option<&secrecy::SecretString> {
    settings.azure_ad_token.as_ref()
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
        2 => loads_azure_tenant_id(),
        3 => loads_azure_client_id(),
        4 => loads_azure_client_secret(),
        5 => loads_azure_username(),
        6 => loads_azure_password(),
        7 => loads_azure_ad_token(),
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

fn loads_azure_tenant_id() {
    assert_eq!(text(&load("", &[]), tenant), None);
    assert_eq!(
        text(&load("      tenant_id: explicit-tenant\n", &[]), tenant).as_deref(),
        Some("explicit-tenant")
    );
    assert_eq!(
        text(
            &load("", &[("AZURE_TENANT_ID", "environment-tenant")]),
            tenant
        )
        .as_deref(),
        Some("environment-tenant")
    );
    assert_eq!(
        text(
            &load(
                "      tenant_id: explicit-tenant\n",
                &[("AZURE_TENANT_ID", "environment-tenant")]
            ),
            tenant
        )
        .as_deref(),
        Some("explicit-tenant")
    );
    assert_eq!(
        text(
            &load(
                "      tenant_id: \"\"\n",
                &[("AZURE_TENANT_ID", "environment-tenant")]
            ),
            tenant
        )
        .as_deref(),
        Some("environment-tenant")
    );
}

fn loads_azure_client_id() {
    assert_eq!(text(&load("", &[]), client), None);
    assert_eq!(
        text(&load("      client_id: explicit-client\n", &[]), client).as_deref(),
        Some("explicit-client")
    );
    assert_eq!(
        text(
            &load("", &[("AZURE_CLIENT_ID", "environment-client")]),
            client
        )
        .as_deref(),
        Some("environment-client")
    );
    assert_eq!(
        text(
            &load(
                "      client_id: explicit-client\n",
                &[("AZURE_CLIENT_ID", "environment-client")]
            ),
            client
        )
        .as_deref(),
        Some("explicit-client")
    );
}

fn loads_azure_client_secret() {
    assert_eq!(secret(&load("", &[]), client_secret), None);
    let settings = load("      client_secret: fixture-client-secret\n", &[]);
    assert_eq!(
        secret(&settings, client_secret).as_deref(),
        Some("fixture-client-secret")
    );
    assert!(!format!("{settings:?}").contains("fixture-client-secret"));
    assert_eq!(
        settings.redact_known("echo fixture-client-secret back"),
        "echo [redacted] back"
    );
    assert_eq!(
        secret(
            &load("", &[("AZURE_CLIENT_SECRET", "environment-secret")]),
            client_secret
        )
        .as_deref(),
        Some("environment-secret")
    );
    assert_eq!(
        secret(
            &load(
                "      client_secret: \"\"\n",
                &[("AZURE_CLIENT_SECRET", "environment-secret")]
            ),
            client_secret
        )
        .as_deref(),
        Some("environment-secret")
    );
}

fn loads_azure_username() {
    assert_eq!(text(&load("", &[]), username), None);
    assert_eq!(
        text(
            &load("      azure_username: explicit-user\n", &[]),
            username
        )
        .as_deref(),
        Some("explicit-user")
    );
    assert_eq!(
        text(
            &load("", &[("AZURE_USERNAME", "environment-user")]),
            username
        )
        .as_deref(),
        Some("environment-user")
    );
}

fn loads_azure_password() {
    assert_eq!(secret(&load("", &[]), password), None);
    let settings = load("      azure_password: fixture-password\n", &[]);
    assert_eq!(
        secret(&settings, password).as_deref(),
        Some("fixture-password")
    );
    assert!(!format!("{settings:?}").contains("fixture-password"));
    assert_eq!(
        settings.redact_known("echo fixture-password back"),
        "echo [redacted] back"
    );
    assert_eq!(
        secret(
            &load("", &[("AZURE_PASSWORD", "environment-password")]),
            password
        )
        .as_deref(),
        Some("environment-password")
    );
}

fn loads_azure_ad_token() {
    assert_eq!(secret(&load("", &[]), ad_token), None);
    let settings = load("      azure_ad_token: fixture-ad-token\n", &[]);
    assert_eq!(
        secret(&settings, ad_token).as_deref(),
        Some("fixture-ad-token")
    );
    assert!(!format!("{settings:?}").contains("fixture-ad-token"));
    assert_eq!(
        settings.redact_known("echo fixture-ad-token back"),
        "echo [redacted] back"
    );
    assert_eq!(
        secret(
            &load("", &[("AZURE_AD_TOKEN", "environment-ad-token")]),
            ad_token
        )
        .as_deref(),
        Some("environment-ad-token")
    );
    assert_eq!(
        secret(
            &load(
                "      azure_ad_token: \"\"\n",
                &[("AZURE_AD_TOKEN", "environment-ad-token")]
            ),
            ad_token
        )
        .as_deref(),
        Some("environment-ad-token")
    );
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
