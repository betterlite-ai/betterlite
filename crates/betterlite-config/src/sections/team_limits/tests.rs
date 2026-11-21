#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use rust_decimal::Decimal;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::team_limits::{
    DefaultInternalUserParams, DefaultTeamParams, INTERNAL_USER_PARAMS_PATH, Settings,
    TEAM_PARAMS_PATH,
};

fn loaded_with(source: &str, lookup: BTreeMap<String, String>) -> Settings {
    let config = ConfigLoader { lookup }.load(source).unwrap();
    config.sections.team_limits
}

fn loaded(source: &str) -> Settings {
    loaded_with(source, BTreeMap::new())
}

fn failed_with(source: &str, lookup: BTreeMap<String, String>) -> ConfigError {
    ConfigLoader { lookup }.load(source).unwrap_err()
}

fn failed(source: &str) -> ConfigError {
    failed_with(source, BTreeMap::new())
}

fn yaml_message(error: ConfigError) -> String {
    let ConfigError::Yaml { source: yaml } = error else {
        panic!("the setting fails the config load as malformed YAML");
    };
    yaml.to_string()
}

fn money(text: &str) -> Decimal {
    text.parse::<Decimal>().unwrap()
}

#[test]
fn default_params_absent() {
    let settings = loaded("litellm_settings: {}\ngeneral_settings: {}\n");
    assert_eq!(settings, Settings::default());
    assert!(settings.default_team_params.is_none());
    assert!(settings.default_internal_user_params.is_none());
    assert!(!settings.disable_admin_auto_add);
    let explicit_null = loaded("litellm_settings:\n  default_team_params: null\n");
    assert!(explicit_null.default_team_params.is_none());
}

#[test]
fn default_params_map() {
    let settings = loaded(
        "\
litellm_settings:
  default_team_params:
    models:
      - gpt-4o
    max_budget: 25.5
    budget_duration: 30d
    tpm_limit: 100
    rpm_limit: 10
    team_member_permissions:
      - /key/generate
    organization_id: org-1
",
    );
    let params = settings.default_team_params.unwrap();
    assert_eq!(params.models, Some(vec!["gpt-4o".to_owned()]));
    assert_eq!(params.max_budget, Some(money("25.5")));
    assert_eq!(params.budget_duration.as_deref(), Some("30d"));
    assert_eq!(params.tpm_limit, Some(100));
    assert_eq!(params.rpm_limit, Some(10));
    assert_eq!(
        params.team_member_permissions,
        Some(vec!["/key/generate".to_owned()])
    );
    assert_eq!(params.organization_id.as_deref(), Some("org-1"));
}

#[test]
fn default_params_minimal() {
    let minimal = loaded("litellm_settings:\n  default_team_params:\n    max_budget: 5.0\n");
    assert_eq!(
        minimal.default_team_params,
        Some(DefaultTeamParams {
            models: None,
            max_budget: Some(money("5.0")),
            budget_duration: None,
            tpm_limit: None,
            rpm_limit: None,
            team_member_permissions: None,
            organization_id: None,
        })
    );
    let nulled = loaded("litellm_settings:\n  default_team_params:\n    max_budget: null\n");
    assert_eq!(
        nulled.default_team_params,
        Some(DefaultTeamParams::default())
    );
}

#[test]
fn default_permission_enums() {
    let settings = loaded(
        "\
litellm_settings:
  default_team_params:
    models: []
    team_member_permissions:
      - /key/update
      - /key/generate
",
    );
    let params = settings.default_team_params.unwrap();
    assert_eq!(params.models, Some(Vec::new()));
    assert_eq!(
        params.team_member_permissions,
        Some(vec!["/key/update".to_owned(), "/key/generate".to_owned()])
    );
    // the dict branch keeps a route the key-route enum does not know as the written string
    let unknown = loaded(
        "litellm_settings:\n  default_team_params:\n    team_member_permissions:\n      - /not/a/key/route\n",
    );
    assert_eq!(
        unknown.default_team_params.unwrap().team_member_permissions,
        Some(vec!["/not/a/key/route".to_owned()])
    );
    let error = failed(
        "litellm_settings:\n  default_team_params:\n    team_member_permissions: /key/generate\n",
    );
    let message = yaml_message(error);
    assert!(message.contains(TEAM_PARAMS_PATH));
    assert!(message.contains("team_member_permissions"));
}

#[test]
fn available_teams_loaded() {
    let settings = loaded(
        "\
litellm_settings:
  default_internal_user_params:
    available_teams:
      - team-a
      - team-b
    max_budget: 10
",
    );
    assert_eq!(
        settings.default_internal_user_params,
        Some(DefaultInternalUserParams {
            available_teams: Some(vec!["team-a".to_owned(), "team-b".to_owned()]),
        })
    );
    let absent_key = loaded(
        "litellm_settings:\n  default_internal_user_params:\n    user_role: internal_user\n",
    );
    assert_eq!(
        absent_key.default_internal_user_params,
        Some(DefaultInternalUserParams {
            available_teams: None,
        })
    );
    let null_value =
        loaded("litellm_settings:\n  default_internal_user_params:\n    available_teams: null\n");
    assert_eq!(
        null_value.default_internal_user_params,
        Some(DefaultInternalUserParams {
            available_teams: None,
        })
    );
}

#[test]
fn available_teams_are_read_from_settings() {
    // the pinned proxy reads the key from litellm_settings only; a misplaced key stays unsupported
    let misplaced = failed(
        "general_settings:\n  default_internal_user_params:\n    available_teams:\n      - team-a\n",
    );
    let ConfigError::Unsupported { paths } = misplaced else {
        panic!("a default_internal_user_params key outside litellm_settings stays unsupported");
    };
    assert_eq!(paths, ["general_settings.default_internal_user_params"]);
    let error = failed(
        "litellm_settings:\n  default_internal_user_params:\n    available_teams: everywhere\n",
    );
    let message = yaml_message(error);
    assert!(message.contains(INTERNAL_USER_PARAMS_PATH));
    assert!(message.contains("available_teams"));
    let not_a_mapping = failed("litellm_settings:\n  default_internal_user_params: later\n");
    assert!(yaml_message(not_a_mapping).contains(INTERNAL_USER_PARAMS_PATH));
}

#[rstest]
#[case::literal_true(
    "general_settings:\n  disable_auto_add_proxy_admin_to_teams: true\n",
    true
)]
#[case::literal_false(
    "general_settings:\n  disable_auto_add_proxy_admin_to_teams: false\n",
    false
)]
#[case::integer(
    "general_settings:\n  disable_auto_add_proxy_admin_to_teams: 1\n",
    false
)]
#[case::text(
    "general_settings:\n  disable_auto_add_proxy_admin_to_teams: \"true\"\n",
    false
)]
#[case::null(
    "general_settings:\n  disable_auto_add_proxy_admin_to_teams: null\n",
    false
)]
#[case::absent("general_settings: {}\n", false)]
fn creator_switch_identity(#[case] source: &str, #[case] expected: bool) {
    assert_eq!(loaded(source).disable_admin_auto_add, expected, "{source}");
}

#[test]
fn env_resolved_defaults() {
    let lookup = BTreeMap::from([
        ("TEAM_BUDGET".to_owned(), "40.5".to_owned()),
        ("TEAM_ORG".to_owned(), "org-env".to_owned()),
        ("TEAM_WINDOW".to_owned(), "30d".to_owned()),
        ("TEAM_TPM".to_owned(), "120".to_owned()),
        ("TEAM_EMPTY".to_owned(), String::new()),
    ]);
    let settings = loaded_with(
        "\
litellm_settings:
  default_team_params:
    max_budget: os.environ/TEAM_BUDGET
    organization_id: os.environ/TEAM_ORG
    budget_duration: os.environ/TEAM_WINDOW
    tpm_limit: os.environ/TEAM_TPM
",
        lookup.clone(),
    );
    let params = settings.default_team_params.unwrap();
    assert_eq!(params.max_budget, Some(money("40.5")));
    assert_eq!(params.organization_id.as_deref(), Some("org-env"));
    assert_eq!(params.budget_duration.as_deref(), Some("30d"));
    assert_eq!(params.tpm_limit, Some(120));
    let empty = loaded_with(
        "litellm_settings:\n  default_team_params:\n    organization_id: os.environ/TEAM_EMPTY\n    budget_duration: os.environ/TEAM_EMPTY\n",
        lookup,
    );
    let empty_params = empty.default_team_params.unwrap();
    assert_eq!(empty_params.organization_id, None);
    assert_eq!(empty_params.budget_duration, None);
}

#[test]
fn env_reference_errors() {
    let lookup = BTreeMap::from([("TEAM_ORG".to_owned(), "org-env".to_owned())]);
    let missing = failed_with(
        "litellm_settings:\n  default_team_params:\n    max_budget: os.environ/TEAM_MISSING\n",
        lookup.clone(),
    );
    let ConfigError::MissingEnvironment { path, .. } = missing else {
        panic!("a missing variable is reported before any value is read");
    };
    assert_eq!(path, "litellm_settings.default_team_params.max_budget");
    let non_numeric = failed_with(
        "litellm_settings:\n  default_team_params:\n    max_budget: os.environ/TEAM_ORG\n",
        lookup,
    );
    assert!(matches!(non_numeric, ConfigError::Environment { .. }));
    let written_text =
        failed("litellm_settings:\n  default_team_params:\n    max_budget: not-a-number\n");
    assert!(matches!(written_text, ConfigError::InvalidScalar { .. }));
}
