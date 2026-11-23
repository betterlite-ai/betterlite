#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

use super::Settings;
use crate::loader::{ConfigError, ConfigLoader};

const ALERT_FRACTION_VARIABLE: &str = "EMAIL_BUDGET_ALERT_MAX_SPEND_ALERT_PERCENTAGE";

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .auth_key_budgets)
}

fn json(source: &str) -> JsonData {
    serde_json::from_str(source).unwrap()
}

#[test]
fn alert_config_default_none() {
    let settings = load("", &[]).unwrap();
    assert_eq!(settings.default_key_max_budget_alert_emails, None);
    let explicit_null = load(
        "litellm_settings:\n  default_key_max_budget_alert_emails:\n",
        &[],
    )
    .unwrap();
    assert_eq!(explicit_null.default_key_max_budget_alert_emails, None);
    let empty_map = load(
        "litellm_settings:\n  default_key_max_budget_alert_emails: {}\n",
        &[],
    )
    .unwrap();
    assert_eq!(
        empty_map.default_key_max_budget_alert_emails,
        Some(BTreeMap::new())
    );
    let unrelated = load("litellm_settings:\n  num_retries: 2\n", &[]).unwrap();
    assert_eq!(unrelated.default_key_max_budget_alert_emails, None);
}

#[test]
fn global_email_map_loaded() {
    let source = "litellm_settings:\n  default_key_max_budget_alert_emails:\n    \"80\":\n      - a@example.com\n      - b@example.com\n    \"90\": c@example.com\n";
    let settings = load(source, &[]).unwrap();
    let config = settings.default_key_max_budget_alert_emails.unwrap();
    assert_eq!(config.len(), 2);
    assert_eq!(config["80"], json("[\"a@example.com\",\"b@example.com\"]"));
    assert_eq!(config["90"], json("\"c@example.com\""));
}

#[test]
fn alert_fraction_default() {
    let settings = load("", &[]).unwrap();
    assert_eq!(settings.alert_fraction.to_bits(), 0.8_f64.to_bits());
    assert_eq!(Settings::default(), settings);
}

#[test]
fn alert_fraction_env_override() {
    for (text, expected) in [
        ("0.5", 0.5_f64),
        ("0", 0.0_f64),
        ("-1.5", -1.5_f64),
        (" 0.25 ", 0.25_f64),
        ("1e-1", 0.1_f64),
        ("inf", f64::INFINITY),
        ("-inf", f64::NEG_INFINITY),
        ("NaN", f64::NAN),
    ] {
        let settings = load("", &[(ALERT_FRACTION_VARIABLE, text)]).unwrap();
        if expected.is_nan() {
            assert!(settings.alert_fraction.is_nan(), "{text}");
        } else {
            assert_eq!(
                settings.alert_fraction.to_bits(),
                expected.to_bits(),
                "{text}"
            );
        }
    }
}

#[test]
fn invalid_alert_fraction_stops_startup() {
    for text in ["", "   ", "eighty", "0.8x"] {
        let error = load("", &[(ALERT_FRACTION_VARIABLE, text)]).unwrap_err();
        assert!(
            matches!(error, ConfigError::InvalidFloat { ref path, .. } if path == ALERT_FRACTION_VARIABLE),
            "{text}: {error}"
        );
    }
}
