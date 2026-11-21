#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_env::EnvError;
use rstest::rstest;

use super::{FeatureSwitches, Settings};
use crate::loader::{ConfigError, ConfigLoader};

fn loaded(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.management_rbac)
}

const OFF: FeatureSwitches = FeatureSwitches {
    disabled: false,
    team_admins_allowed: false,
};

#[rstest]
#[case::both_on("true", "true", true, true)]
#[case::disabled_only("true", "false", true, false)]
#[case::allowed_only("false", "true", false, true)]
#[case::both_off("false", "false", false, false)]
fn loads_agents_switches(
    #[case] disabled: &str,
    #[case] allowed: &str,
    #[case] disabled_state: bool,
    #[case] allowed_state: bool,
) {
    let source = format!(
        "general_settings:\n  disable_agents_for_internal_users: {disabled}\n  allow_agents_for_team_admins: {allowed}\n"
    );
    let expected = Settings {
        agents: FeatureSwitches {
            disabled: disabled_state,
            team_admins_allowed: allowed_state,
        },
        vector_stores: OFF,
    };
    assert_eq!(loaded(&source, &[]).unwrap(), expected);
}

#[rstest]
#[case::both_on("true", "true", true, true)]
#[case::disabled_only("true", "false", true, false)]
#[case::allowed_only("false", "true", false, true)]
#[case::both_off("false", "false", false, false)]
fn loads_vector_store_switches(
    #[case] disabled: &str,
    #[case] allowed: &str,
    #[case] disabled_state: bool,
    #[case] allowed_state: bool,
) {
    let source = format!(
        "general_settings:\n  disable_vector_stores_for_internal_users: {disabled}\n  allow_vector_stores_for_team_admins: {allowed}\n"
    );
    let expected = Settings {
        agents: OFF,
        vector_stores: FeatureSwitches {
            disabled: disabled_state,
            team_admins_allowed: allowed_state,
        },
    };
    assert_eq!(loaded(&source, &[]).unwrap(), expected);
}

#[rstest]
#[case::no_general_settings("model_list: []\n")]
#[case::empty_general_settings("general_settings: {}\n")]
#[case::every_key_null(
    "general_settings:\n  disable_agents_for_internal_users: null\n  allow_agents_for_team_admins: null\n  disable_vector_stores_for_internal_users: null\n  allow_vector_stores_for_team_admins: null\n"
)]
fn all_switches_default_false(#[case] source: &str) {
    let expected = Settings {
        agents: OFF,
        vector_stores: OFF,
    };
    assert_eq!(loaded(source, &[]).unwrap(), expected);
}

#[rstest]
#[case::boolean_true("true", true)]
#[case::boolean_false("false", false)]
#[case::null("null", false)]
#[case::one("1", true)]
#[case::zero("0", false)]
#[case::negative("-1", true)]
#[case::float_nonzero("0.5", true)]
#[case::float_zero("0.0", false)]
#[case::text_false("\"false\"", true)]
#[case::text_zero("\"0\"", true)]
#[case::empty_text("\"\"", false)]
#[case::empty_list("[]", false)]
#[case::one_item_list("[false]", true)]
#[case::empty_mapping("{}", false)]
#[case::one_entry_mapping("{a: 1}", true)]
fn runtime_truthiness(#[case] value: &str, #[case] expected: bool) {
    let source = format!(
        "general_settings:\n  disable_agents_for_internal_users: {value}\n  allow_agents_for_team_admins: {value}\n  disable_vector_stores_for_internal_users: {value}\n  allow_vector_stores_for_team_admins: {value}\n"
    );
    let switches = FeatureSwitches {
        disabled: expected,
        team_admins_allowed: expected,
    };
    let settings = loaded(&source, &[]).unwrap();
    assert_eq!(settings.agents, switches);
    assert_eq!(settings.vector_stores, switches);
}

#[rstest]
#[case::true_word(&[("B864_SWITCH", "true")], true)]
#[case::false_word(&[("B864_SWITCH", "false")], false)]
#[case::padded_upper(&[("B864_SWITCH", " TRUE ")], true)]
#[case::zero_word(&[("B864_SWITCH", "0")], false)]
#[case::empty_variable(&[("B864_SWITCH", "")], false)]
#[case::unset_variable(&[], false)]
fn environment_reference_switches(#[case] variables: &[(&str, &str)], #[case] expected: bool) {
    let source = "general_settings:\n  disable_vector_stores_for_internal_users: os.environ/B864_SWITCH\n  allow_agents_for_team_admins: os.environ/B864_SWITCH\n";
    let settings = loaded(source, variables).unwrap();
    assert_eq!(settings.vector_stores.disabled, expected);
    assert_eq!(settings.agents.team_admins_allowed, expected);
    assert!(!settings.agents.disabled);
    assert!(!settings.vector_stores.team_admins_allowed);
}

#[test]
fn environment_reference_rejects_an_unknown_word() {
    let source = "general_settings:\n  disable_agents_for_internal_users: os.environ/B864_SWITCH\n";
    let refused = loaded(source, &[("B864_SWITCH", "maybe")]);
    assert!(matches!(
        refused,
        Err(ConfigError::Environment {
            source: EnvError::Bool { name },
        }) if name == "B864_SWITCH"
    ));
}
