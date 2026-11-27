#![cfg(test)]

use std::collections::BTreeMap;

use crate::loader::ConfigLoader;
use crate::sections::health_services::Settings;

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<Settings, String> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.health_services)
        .map_err(|error| error.to_string())
}

const ALERTING: &str = "general_settings:\n  alerting:\n    - slack\n    - ms_teams\n";

#[test]
fn a_config_without_the_env_var_keeps_the_email_address_unset() {
    let resolved = settings(ALERTING, &[]).unwrap();
    assert_eq!(resolved.test_email_address, None);
}

#[test]
fn the_test_email_address_is_read_from_the_env() {
    let resolved = settings(ALERTING, &[("TEST_EMAIL_ADDRESS", "ops@example.com")]).unwrap();
    assert_eq!(
        resolved.test_email_address.as_deref(),
        Some("ops@example.com")
    );
}

#[test]
fn an_empty_test_email_address_counts_as_unset() {
    let resolved = settings(ALERTING, &[("TEST_EMAIL_ADDRESS", "")]).unwrap();
    assert_eq!(resolved.test_email_address, None);
}

#[test]
fn a_single_space_test_email_address_is_kept() {
    let resolved = settings(ALERTING, &[("TEST_EMAIL_ADDRESS", " ")]).unwrap();
    assert_eq!(resolved.test_email_address.as_deref(), Some(" "));
}

#[test]
fn alerting_is_read_from_the_alert_settings_section() {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(ALERTING)
    .unwrap();
    assert_eq!(
        config.sections.alert_settings.alerting,
        vec!["slack".to_owned(), "ms_teams".to_owned()]
    );
    assert_eq!(config.sections.health_services.test_email_address, None);
}

#[test]
fn a_config_without_general_settings_has_no_alerting() {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("model_list: []\n")
    .unwrap();
    assert!(config.sections.alert_settings.alerting.is_empty());
}

#[test]
fn the_shipped_valid_seed_loads() {
    let source = include_str!("../../../fuzz/corpus/config_yaml/health-services-valid.yaml");
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap();
    assert_eq!(
        config.sections.alert_settings.alerting,
        vec!["slack".to_owned(), "ms_teams".to_owned()]
    );
    assert_eq!(config.sections.health_services.test_email_address, None);
}

#[test]
fn the_shipped_invalid_seed_is_refused() {
    let source = include_str!("../../../fuzz/corpus/config_yaml/health-services-invalid.yaml");
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap_err();
    assert!(!error.to_string().is_empty());
}
