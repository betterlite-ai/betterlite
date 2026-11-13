#![cfg(test)]

use super::CoreSettings;
use betterlite_config::loader::{ConfigError, ConfigLoader};
use rstest::rstest;
use std::collections::BTreeMap;

#[rstest]
#[case::default(None, 6000.0_f64, false)]
#[case::explicit(Some("12.5"), 12.5_f64, true)]
#[case::zero(Some("0"), 0.0_f64, true)]
#[case::empty_unset(Some(""), 6000.0_f64, false)]
#[case::exponent(Some("1e2"), 100.0_f64, true)]
#[case::whitespace(Some(" 3.5 "), 3.5_f64, true)]
fn call_context_tests_behavior(
    #[case] raw: Option<&str>,
    #[case] seconds: f64,
    #[case] explicit: bool,
) {
    let mut environment = raw
        .map(|value| BTreeMap::from([("REQUEST_TIMEOUT".to_owned(), value.to_owned())]))
        .unwrap_or_default();
    environment.extend([
        ("YAML_TIMEOUT".to_owned(), "9".to_owned()),
        ("MODEL_TIMEOUT".to_owned(), "2".to_owned()),
    ]);
    let loader = ConfigLoader {
        lookup: environment,
    };
    let config = loader.load("litellm_settings:\n  request_timeout: os.environ/YAML_TIMEOUT\nmodel_list:\n  - model_name: one\n    litellm_params:\n      timeout: os.environ/MODEL_TIMEOUT\n  - model_name: two\n    litellm_params: {}\n").unwrap();
    let settings = CoreSettings::load(&config, &loader.lookup).unwrap();
    assert_eq!(
        settings.request_timeout_seconds.to_bits(),
        seconds.to_bits()
    );
    assert_eq!(settings.request_timeout_explicit, explicit);
    assert_eq!(settings.litellm_request_timeout, Some(9.0_f64));
    assert_eq!(settings.model_timeout_overrides, vec![Some(2.0_f64), None]);
    let other = loader.load("{}").unwrap();
    let other_settings = CoreSettings::load(&other, &BTreeMap::new()).unwrap();
    assert_eq!(
        other_settings.request_timeout_seconds.to_bits(),
        6000.0_f64.to_bits()
    );
    assert_eq!(other_settings.litellm_request_timeout, None);
    assert!(other_settings.model_timeout_overrides.is_empty());
    assert_eq!(settings.litellm_request_timeout, Some(9.0_f64));
}

mod failures;
