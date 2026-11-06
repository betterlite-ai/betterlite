use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use secrecy::{ExposeSecret as _, SecretString};

use super::support::{BASE, KEY, settings};
use crate::braintrust_logging::logger::BraintrustLogger;
use crate::braintrust_logging::settings::{Settings, SettingsError};
use crate::http_analytics_loggers::delivery::PooledDelivery;

fn environment(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn init_builds_the_bearer_headers_from_the_key() {
    let settings = settings(&[]);
    assert_eq!(settings.api_key.expose_secret(), KEY);
    assert_eq!(settings.api_base, BASE);
}

#[test]
fn init_reads_the_key_from_the_environment() {
    let variables = environment(&[("BRAINTRUST_API_KEY", "env-key")]);
    let resolved = Settings::resolve(None, None, Vec::new(), &variables).unwrap();
    assert_eq!(resolved.api_key.expose_secret(), "env-key");
    assert_eq!(resolved.api_base, "https://api.braintrustdata.com/v1");
}

#[test]
fn init_prefers_the_argument_over_the_environment() {
    let variables = environment(&[
        ("BRAINTRUST_API_KEY", "env-key"),
        ("BRAINTRUST_API_BASE", "https://env.example/v1"),
    ]);
    let resolved = Settings::resolve(
        Some(SecretString::from("arg-key")),
        Some("https://arg.example/v1".to_owned()),
        vec!["user_id".to_owned()],
        &variables,
    )
    .unwrap();
    assert_eq!(resolved.api_key.expose_secret(), "arg-key");
    assert_eq!(resolved.api_base, "https://arg.example/v1");
    assert_eq!(resolved.default_tags, ["user_id"]);
}

#[test]
fn init_treats_empty_environment_values_as_unset() {
    let variables = environment(&[("BRAINTRUST_API_KEY", ""), ("BRAINTRUST_API_BASE", "")]);
    let missing = Settings::resolve(None, None, Vec::new(), &variables);
    assert_eq!(missing.unwrap_err(), SettingsError::MissingApiKey);
    let resolved = Settings::resolve(
        Some(SecretString::from("arg-key")),
        None,
        Vec::new(),
        &variables,
    )
    .unwrap();
    assert_eq!(resolved.api_base, "https://api.braintrustdata.com/v1");
}

#[test]
fn init_debug_output_hides_the_key() {
    let settings = settings(&[]);
    assert!(!format!("{settings:?}").contains(KEY));
}

#[test]
fn init_composes_with_the_pooled_http_delivery() {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    let client = ProxiedClient::outbound(&Arc::new(policy)).unwrap();
    let logger = BraintrustLogger::new(settings(&[]), PooledDelivery { client });
    logger.log_failure_event();
}
