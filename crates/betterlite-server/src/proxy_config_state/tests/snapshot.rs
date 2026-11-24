use betterlite_config::sections::settings::{JsonValue, Section};

use super::support::{self, json};
use crate::proxy_config_state::error::ConfigStateError;
use crate::proxy_config_state::snapshot::ConfigWithBaseline;

#[test]
fn a_snapshot_is_a_deep_copy() {
    let mut state = support::state();
    let mut config = support::values(&[("general_settings", json!({"nested": {"a": 1}}))]);
    state.replace_snapshot(&config);
    let Some(JsonValue::Object(entries)) = config.get_mut("general_settings") else {
        panic!("the config keeps its object");
    };
    entries.insert("nested".to_owned(), json!({"a": 2}));
    assert_eq!(
        state.snapshot().get("general_settings"),
        Some(&json!({"nested": {"a": 1}}))
    );

    let mut taken = state.snapshot();
    taken.insert("general_settings".to_owned(), json!({"nested": {"a": 3}}));
    assert_eq!(
        taken.get("general_settings"),
        Some(&json!({"nested": {"a": 3}}))
    );
    assert_eq!(
        state.snapshot().get("general_settings"),
        Some(&json!({"nested": {"a": 1}}))
    );
}

#[test]
fn a_replaced_snapshot_keeps_the_callers_next_change_out() {
    let mut state = support::state();
    let config = support::values(&[("general_settings", json!({"a": 1}))]);
    state.replace_snapshot(&config);
    assert_eq!(state.snapshot(), config);
}

#[test]
fn a_config_given_only_values_has_no_baseline() {
    let config = ConfigWithBaseline::from_values(support::values(&[("a", json!(1))]));
    assert!(config.baseline.as_ref().is_none());
    assert_eq!(config.values, support::values(&[("a", json!(1))]));
}

#[test]
fn a_config_with_a_baseline_keeps_the_values_and_carries_the_baseline() {
    let baseline = support::values(&[("a", json!(2))]);
    let config =
        ConfigWithBaseline::with_baseline(support::values(&[("a", json!(1))]), baseline.clone());
    assert_eq!(config.baseline.as_ref(), Some(&baseline));
    assert_eq!(config.values, support::values(&[("a", json!(1))]));
}

#[test]
fn a_config_owned_error_reports_its_first_detail_line() {
    let mut state = support::state();
    state
        .general_settings
        .load_yaml(support::values(&[("master_key", json!("sk-file"))]));
    let changed = support::values(&[("master_key", json!("sk-new"))]);
    let error = state
        .reject_config_owned_writes(Section::GeneralSettings, &changed)
        .unwrap_err();
    assert!(matches!(error, ConfigStateError::ConfigOwned(_)));
    assert_eq!(
        error.to_string(),
        "general_settings key 'master_key' is set in the config file and cannot be changed here."
    );
}
