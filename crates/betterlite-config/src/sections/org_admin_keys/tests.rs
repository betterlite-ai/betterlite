#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_env::EnvError;
use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

fn settings(source: &str, variables: &[(&str, &str)]) -> Result<bool, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .map(|config| config.sections.org_admin_keys.key_generation_disabled)
}

#[rstest]
#[case::on("true", true)]
#[case::off("false", false)]
fn loads_org_key_switch(#[case] value: &str, #[case] expected: bool) {
    let source = format!("general_settings:\n  disable_key_generate_for_org_admin: {value}\n");
    assert_eq!(settings(&source, &[]).unwrap(), expected);
}

#[rstest]
#[case::absent_key("model_list: []\n")]
#[case::empty_general_settings("general_settings: {}\n")]
#[case::null_value("general_settings:\n  disable_key_generate_for_org_admin: null\n")]
fn org_key_switch_defaults_false(#[case] source: &str) {
    assert!(!settings(source, &[]).unwrap());
}

#[rstest]
#[case::one("1", true)]
#[case::zero("0", false)]
#[case::float_nonzero("1.5", true)]
#[case::float_zero("0.0", false)]
#[case::text_false("\"false\"", true)]
#[case::text_zero("\"0\"", true)]
#[case::empty_text("\"\"", false)]
#[case::empty_list("[]", false)]
#[case::one_item_list("[0]", true)]
#[case::empty_mapping("{}", false)]
#[case::one_entry_mapping("{a: 1}", true)]
fn org_key_runtime_truthiness(#[case] value: &str, #[case] expected: bool) {
    let source = format!("general_settings:\n  disable_key_generate_for_org_admin: {value}\n");
    assert_eq!(settings(&source, &[]).unwrap(), expected);
}

#[rstest]
#[case::true_word(&[("B10015_SETTING", "true")], true)]
#[case::false_word(&[("B10015_SETTING", "false")], false)]
#[case::no_word(&[("B10015_SETTING", "no")], false)]
#[case::zero_word(&[("B10015_SETTING", "0")], false)]
#[case::padded_case(&[("B10015_SETTING", " FALSE ")], false)]
#[case::empty_environment(&[("B10015_SETTING", "")], false)]
#[case::unset_environment(&[], false)]
fn org_key_environment_reference_truthiness(
    #[case] variables: &[(&str, &str)],
    #[case] expected: bool,
) {
    let source =
        "general_settings:\n  disable_key_generate_for_org_admin: os.environ/B10015_SETTING\n";
    assert_eq!(settings(source, variables).unwrap(), expected);
}

#[test]
fn org_key_environment_reference_rejects_an_unrecognized_word() {
    let source =
        "general_settings:\n  disable_key_generate_for_org_admin: os.environ/B10015_SETTING\n";
    let refused = settings(source, &[("B10015_SETTING", "maybe")]);
    assert!(matches!(
        refused,
        Err(ConfigError::Environment {
            source: EnvError::Bool { name },
        }) if name == "B10015_SETTING"
    ));
}
