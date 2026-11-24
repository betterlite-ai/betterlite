#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;

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
    assert_eq!(settings.default_alert_emails, None);
    let explicit_null = load(
        "litellm_settings:\n  default_key_max_budget_alert_emails:\n",
        &[],
    )
    .unwrap();
    assert_eq!(explicit_null.default_alert_emails, None);
    let empty_map = load(
        "litellm_settings:\n  default_key_max_budget_alert_emails: {}\n",
        &[],
    )
    .unwrap();
    assert_eq!(empty_map.default_alert_emails, Some(BTreeMap::new()));
    let unrelated = load("litellm_settings:\n  num_retries: 2\n", &[]).unwrap();
    assert_eq!(unrelated.default_alert_emails, None);
}

#[test]
fn global_email_map_loaded() {
    let source = "litellm_settings:\n  default_key_max_budget_alert_emails:\n    \"80\":\n      - a@example.com\n      - b@example.com\n    \"90\": c@example.com\n";
    let settings = load(source, &[]).unwrap();
    let config = settings.default_alert_emails.unwrap();
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

#[rstest]
#[case::half("0.5", 0.5_f64)]
#[case::zero("0", 0.0_f64)]
#[case::negative("-1.5", -1.5_f64)]
#[case::trimmed(" 0.25 ", 0.25_f64)]
#[case::exponent("1e-1", 0.1_f64)]
#[case::infinity("inf", f64::INFINITY)]
#[case::negative_infinity("-inf", f64::NEG_INFINITY)]
#[case::nan("NaN", f64::NAN)]
fn alert_fraction_env_override(#[case] text: &str, #[case] expected: f64) {
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

#[rstest]
#[case::empty("")]
#[case::blank("   ")]
#[case::word("eighty")]
#[case::suffix("0.8x")]
fn invalid_alert_fraction_stops_startup(#[case] text: &str) {
    let error = load("", &[(ALERT_FRACTION_VARIABLE, text)]).unwrap_err();
    assert!(
        matches!(&error, ConfigError::InvalidFloat { path, .. } if path == ALERT_FRACTION_VARIABLE),
        "{text}: {error}"
    );
}
