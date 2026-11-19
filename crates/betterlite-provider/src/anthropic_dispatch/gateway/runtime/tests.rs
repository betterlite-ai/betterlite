#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_anthropic::credentials::AnthropicSettings;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::{Config, ProxyTrust};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::ModelCatalog;
use secrecy::ExposeSecret as _;

use super::{AnthropicRuntime, credentials, environment, option_defaults, settings};

fn config(lookup: &[(&str, &str)], yaml: &str) -> Config {
    let environment: BTreeMap<String, String> = lookup
        .iter()
        .map(|(name, value)| (String::from(*name), String::from(*value)))
        .collect();
    ConfigLoader {
        lookup: environment,
    }
    .load(yaml)
    .expect("the test config loads")
}

#[test]
fn the_settings_read_the_anthropic_credential_environment() {
    let config = config(
        &[
            ("ANTHROPIC_API_KEY", "sk-anthropic"),
            ("ANTHROPIC_AUTH_TOKEN", "auth-token"),
            ("ANTHROPIC_BASE_URL", "https://anthropic.example"),
        ],
        "litellm_settings:\n  use_bearer_for_custom_base: true\n",
    );
    let settings = settings(&config);
    let api_key = settings
        .api_key
        .as_ref()
        .map(|key| key.expose_secret().to_owned());
    let auth_token = settings
        .auth_token
        .as_ref()
        .map(|token| token.expose_secret().to_owned());
    assert_eq!(api_key, Some(String::from("sk-anthropic")));
    assert_eq!(auth_token, Some(String::from("auth-token")));
    assert_eq!(settings.api_base, "https://anthropic.example");
    assert!(settings.use_bearer_for_custom_base);
    assert_eq!(
        settings.anthropic_version,
        AnthropicSettings::default().anthropic_version
    );
}

#[test]
fn the_credentials_read_the_sdk_settings() {
    let config = config(
        &[],
        "litellm_settings:\n  api_key: sk-global\n  api_base: https://global.example\n",
    );
    let credentials = credentials(&config);
    let api_key = credentials
        .api_key
        .as_ref()
        .map(|key| key.expose_secret().to_owned());
    assert_eq!(api_key, Some(String::from("sk-global")));
    assert_eq!(
        credentials.api_base.as_deref(),
        Some("https://global.example")
    );
    assert!(credentials.xai_key.is_none());
}

#[test]
fn the_environment_holds_the_configured_variables() {
    let config = config(&[], "environment_variables:\n  REGION: eu\n");
    let environment = environment(&config);
    assert_eq!(
        environment,
        BTreeMap::from([(String::from("REGION"), String::from("eu"))])
    );
}

#[test]
fn the_option_defaults_read_the_option_values() {
    let config = config(
        &[],
        "litellm_settings:\n  max_tokens: 42\n  temperature: 0.25\n  top_k: 7\n  system: sys\n",
    );
    let defaults = option_defaults(&config.sections.anthropic_option_primitives.litellm_settings);
    assert_eq!(defaults.max_tokens, Some(42));
    assert_eq!(
        defaults
            .temperature
            .as_ref()
            .and_then(serde_json::Number::as_f64),
        Some(0.25)
    );
    assert_eq!(defaults.top_k, Some(7));
    assert_eq!(defaults.system.as_deref(), Some("sys"));
}

#[test]
fn the_runtime_debug_view_names_only_the_type() {
    let config = config(&[], "litellm_settings: {}\n");
    let policy =
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).expect("the proxy policy");
    let client = ProxiedClient::outbound(&Arc::new(policy)).expect("the client");
    let runtime = AnthropicRuntime::new(&config, &client, Arc::new(ModelCatalog::default()))
        .expect("the runtime");
    assert_eq!(format!("{runtime:?}"), "AnthropicRuntime { .. }");
}
