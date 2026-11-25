use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;

use super::support::{self, json};
use crate::proxy_config_state::error::ConfigStateError;
use crate::proxy_config_state::state::ProxyConfigState;
use crate::proxy_config_state::team;

fn state_with_teams(teams: JsonValue) -> ProxyConfigState {
    let mut state = support::state();
    let settings = JsonValue::Object(
        support::values(&[("default_team_settings", teams)])
            .into_iter()
            .collect(),
    );
    state.replace_snapshot(&support::values(&[("litellm_settings", settings)]));
    state
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn team_config_finds_the_team_and_resolves_environment_references() {
    let state = state_with_teams(json!([
        {"team_id": "other", "api_key": "sk-other"},
        {
            "team_id": "team-1",
            "api_key": "os.environ/TEAM_KEY",
            "drop_params": "os.environ/DROP_PARAMS",
            "model": "gpt-4o"
        }
    ]));
    let environment = environment(&[("TEAM_KEY", "sk-env"), ("DROP_PARAMS", " TRUE ")]);
    let config = team::team_config(&state, "team-1", &environment).unwrap();
    assert_eq!(
        config,
        support::values(&[
            ("team_id", json!("team-1")),
            ("api_key", json!("sk-env")),
            ("drop_params", json!(true)),
            ("model", json!("gpt-4o")),
        ])
    );
}

#[test]
fn a_reference_to_a_missing_variable_resolves_to_null() {
    let state = state_with_teams(json!([
        {"team_id": "team-1", "api_key": "os.environ/ABSENT", "model": "os.environ/MODEL"}
    ]));
    let environment = environment(&[("MODEL", "gpt-4o")]);
    let config = team::team_config(&state, "team-1", &environment).unwrap();
    assert_eq!(
        config,
        support::values(&[
            ("team_id", json!("team-1")),
            ("api_key", JsonValue::Null),
            ("model", json!("gpt-4o")),
        ])
    );
}

#[test]
fn a_false_environment_value_becomes_a_bool() {
    let state = state_with_teams(json!([
        {"team_id": "team-1", "drop_params": "os.environ/DROP_PARAMS"}
    ]));
    let environment = environment(&[("DROP_PARAMS", "false")]);
    let config = team::team_config(&state, "team-1", &environment).unwrap();
    assert_eq!(config.get("drop_params"), Some(&json!(false)));
}

#[test]
fn an_entry_without_a_team_id_is_refused_with_its_secrets_masked() {
    let state = state_with_teams(json!([
        {"api_key": "sk-very-secret-value", "mode": "chat"}
    ]));
    let error = team::team_config(&state, "team-1", &environment(&[])).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("team_id missing from team"));
    assert!(!message.contains("sk-very-secret-value"));
    let ConfigStateError::MissingTeamId { team } = error else {
        panic!("expected a missing-team-id error");
    };
    let shown = team.to_string();
    assert!(shown.contains('*'));
    assert!(!shown.contains("sk-very-secret-value"));
}

#[test]
fn an_entry_that_is_not_an_object_is_refused() {
    let state = state_with_teams(json!(["team-1"]));
    let error = team::team_config(&state, "team-1", &environment(&[])).unwrap_err();
    assert!(matches!(error, ConfigStateError::MissingTeamId { .. }));
}

#[test]
fn a_config_without_default_team_settings_has_no_team_config() {
    let state = support::state();
    let config = team::team_config(&state, "team-1", &environment(&[])).unwrap();
    assert_eq!(config, BTreeMap::new());
}

#[test]
fn an_unknown_team_has_no_team_config() {
    let state = state_with_teams(json!([{"team_id": "team-1", "model": "gpt-4o"}]));
    let config = team::team_config(&state, "team-2", &environment(&[])).unwrap();
    assert_eq!(config, BTreeMap::new());
}

#[test]
fn default_team_settings_that_are_not_a_list_are_rejected() {
    let state = state_with_teams(json!({"team_id": "team-1"}));
    let error = team::team_config(&state, "team-1", &environment(&[])).unwrap_err();
    let ConfigStateError::InvalidSetting { field, expected } = error else {
        panic!("expected an invalid-setting error");
    };
    assert_eq!(field, "litellm_settings.default_team_settings");
    assert_eq!(expected, "a list");
}
