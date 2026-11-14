#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::ConfigLoader;
use crate::sections::copilot_chat::Settings;

fn load(document: &str, environment: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(document)
        .expect("the copilot chat settings load")
        .sections
        .copilot_chat
}

#[rstest]
#[case::absent("", &[])]
#[case::explicit_false("litellm_settings:\n  disable_copilot_system_to_assistant: false\n", &[])]
fn copilot_chat_settings_default_to_conversion_and_no_base(
    #[case] document: &str,
    #[case] environment: &[(&str, &str)],
) {
    let settings = load(document, environment);

    assert!(!settings.disable_system_to_assistant);
    assert_eq!(settings.api_base, None);
}

#[test]
fn copilot_chat_settings_take_the_yaml_flag_and_the_environment_base() {
    let settings = load(
        "litellm_settings:\n  disable_copilot_system_to_assistant: true\n",
        &[("GITHUB_COPILOT_API_BASE", "https://copilot.example/v1")],
    );

    assert!(settings.disable_system_to_assistant);
    assert_eq!(
        settings.api_base.as_deref(),
        Some("https://copilot.example/v1")
    );
}

#[test]
fn copilot_chat_settings_treat_an_empty_base_as_unset() {
    let settings = load("", &[("GITHUB_COPILOT_API_BASE", "")]);

    assert_eq!(settings.api_base, None);
}

#[test]
fn copilot_chat_settings_reject_a_non_boolean_flag() {
    let lookup = BTreeMap::new();
    let error = ConfigLoader { lookup }
        .load("litellm_settings:\n  disable_copilot_system_to_assistant: yes please\n")
        .expect_err("a non-boolean flag is a load error");

    assert!(error.to_string().contains("invalid YAML"), "{error}");
}
