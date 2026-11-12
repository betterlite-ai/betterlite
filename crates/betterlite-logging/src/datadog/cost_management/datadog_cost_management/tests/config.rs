use std::collections::BTreeMap;
use std::time::Duration;

use secrecy::ExposeSecret;

use super::support;
use crate::datadog::cost_management::datadog_cost_management::settings::{
    CostManagementInit, CostManagementSettings,
};

fn environment_with(extra: &[(&str, &str)]) -> BTreeMap<String, String> {
    let mut environment = support::environment();
    for (name, value) in extra {
        environment.insert((*name).to_owned(), (*value).to_owned());
    }
    environment
}

#[test]
fn config_reads_the_api_keys_site_and_missing_key_warning() {
    let settings = support::settings(&support::init(&[]));
    assert_eq!(
        settings.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("api-key")
    );
    assert_eq!(
        settings.app_key.as_ref().map(ExposeSecret::expose_secret),
        Some("app-key")
    );
    assert_eq!(settings.site, "test.datadoghq.com");
}

#[test]
fn config_defaults_the_site_and_warns_when_a_key_is_missing() {
    let environment = BTreeMap::from([("DD_API_KEY".to_owned(), "api-key".to_owned())]);
    let (resolved, logs) = support::logged(|| {
        CostManagementSettings::resolve(&CostManagementInit::default(), &environment)
    });
    let settings = resolved.expect("the settings resolve");
    assert_eq!(settings.site, "datadoghq.com");
    assert!(settings.app_key.is_none());
    assert!(logs.contains("DD_API_KEY and DD_APP_KEY are required"));
}

#[test]
fn config_reads_an_empty_credential_as_missing() {
    let environment = environment_with(&[("DD_API_KEY", "")]);
    let (resolved, logs) = support::logged(|| {
        CostManagementSettings::resolve(&CostManagementInit::default(), &environment)
    });
    let settings = resolved.expect("the settings resolve");
    assert!(settings.api_key.is_none());
    assert!(settings.app_key.is_some());
    assert!(logs.contains("DD_API_KEY and DD_APP_KEY are required"));
}

#[test]
fn config_reads_the_batch_defaults_from_the_environment() {
    let environment = environment_with(&[
        ("DEFAULT_BATCH_SIZE", "64"),
        ("DEFAULT_FLUSH_INTERVAL_SECONDS", "9"),
    ]);
    let settings = support::settings_with(&CostManagementInit::default(), &environment);
    assert_eq!(settings.batch.batch_size, 64);
    assert_eq!(settings.batch.flush_interval, Duration::from_secs(9));
}

#[test]
fn config_lets_the_callback_settings_override_the_batch_defaults() {
    let environment = environment_with(&[("DEFAULT_BATCH_SIZE", "64")]);
    let mut init = support::init(&[]);
    init.batch_size = Some(4);
    init.max_queue_size = Some(20);
    let settings = support::settings_with(&init, &environment);
    assert_eq!(settings.batch.batch_size, 4);
    assert_eq!(settings.batch.max_queue_size, 20);
}
