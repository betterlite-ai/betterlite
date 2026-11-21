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
const ON: FeatureSwitches = FeatureSwitches {
    disabled: true,
    team_admins_allowed: true,
};
const DISABLED_ONLY: FeatureSwitches = FeatureSwitches {
    disabled: true,
    team_admins_allowed: false,
};
const ALLOWED_ONLY: FeatureSwitches = FeatureSwitches {
    disabled: false,
    team_admins_allowed: true,
};

const AGENT_KEYS: (&str, &str) = (
    "disable_agents_for_internal_users",
    "allow_agents_for_team_admins",
);
const VECTOR_STORE_KEYS: (&str, &str) = (
    "disable_vector_stores_for_internal_users",
    "allow_vector_stores_for_team_admins",
);

fn two_keys(keys: (&str, &str), values: (&str, &str)) -> Settings {
    let source = format!(
        "general_settings:\n  {}: {}\n  {}: {}\n",
        keys.0, values.0, keys.1, values.1
    );
    loaded(&source, &[]).unwrap()
}

#[rstest]
#[case::both_on(("true", "true"), ON)]
#[case::disabled_only(("true", "false"), DISABLED_ONLY)]
#[case::allowed_only(("false", "true"), ALLOWED_ONLY)]
#[case::both_off(("false", "false"), OFF)]
fn loads_agents_switches(#[case] values: (&str, &str), #[case] agents: FeatureSwitches) {
    let settings = two_keys(AGENT_KEYS, values);
    assert_eq!((settings.agents, settings.vector_stores), (agents, OFF));
}

#[rstest]
#[case::both_on(("true", "true"), ON)]
#[case::disabled_only(("true", "false"), DISABLED_ONLY)]
#[case::allowed_only(("false", "true"), ALLOWED_ONLY)]
#[case::both_off(("false", "false"), OFF)]
fn loads_vector_store_switches(#[case] values: (&str, &str), #[case] stores: FeatureSwitches) {
    let settings = two_keys(VECTOR_STORE_KEYS, values);
    assert_eq!((settings.agents, settings.vector_stores), (OFF, stores));
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
