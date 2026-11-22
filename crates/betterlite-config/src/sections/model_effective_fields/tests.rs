#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::Settings;

fn load(source: &str, variables: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .model_effective_fields)
}

#[rstest]
#[case::enabled("general_settings: {enforce_rpm_tpm_on_model_add: true}", &[], true)]
#[case::disabled("general_settings: {enforce_rpm_tpm_on_model_add: false}", &[], false)]
#[case::env_true(
    "general_settings: {enforce_rpm_tpm_on_model_add: os.environ/ENFORCE_LIMITS}",
    &[("ENFORCE_LIMITS", "true")],
    true
)]
#[case::env_spelled_on(
    "general_settings: {enforce_rpm_tpm_on_model_add: os.environ/ENFORCE_LIMITS}",
    &[("ENFORCE_LIMITS", " ON ")],
    true
)]
#[case::env_false(
    "general_settings: {enforce_rpm_tpm_on_model_add: os.environ/ENFORCE_LIMITS}",
    &[("ENFORCE_LIMITS", "false")],
    false
)]
fn loads_settings(
    #[case] source: &str,
    #[case] variables: &[(&str, &str)],
    #[case] expected: bool,
) {
    let settings = load(source, variables).unwrap();
    assert_eq!(settings.enforce_rpm_tpm_on_model_add, expected);
}

#[rstest]
#[case::absent_section("{}", &[])]
#[case::absent_key("general_settings: {master_key: sk-1234}", &[])]
#[case::null("general_settings: {enforce_rpm_tpm_on_model_add: null}", &[])]
#[case::unset_reference(
    "general_settings: {enforce_rpm_tpm_on_model_add: os.environ/ENFORCE_LIMITS}",
    &[]
)]
#[case::empty_reference(
    "general_settings: {enforce_rpm_tpm_on_model_add: os.environ/ENFORCE_LIMITS}",
    &[("ENFORCE_LIMITS", "  ")]
)]
#[case::literal_beats_a_same_named_variable(
    "general_settings: {enforce_rpm_tpm_on_model_add: false}",
    &[("ENFORCE_RPM_TPM_ON_MODEL_ADD", "true")]
)]
#[case::unreferenced_variable_is_ignored("{}", &[("ENFORCE_RPM_TPM_ON_MODEL_ADD", "true")])]
fn defaults_and_precedence(#[case] source: &str, #[case] variables: &[(&str, &str)]) {
    let settings = load(source, variables).unwrap();
    assert!(!settings.enforce_rpm_tpm_on_model_add);
}

#[test]
fn invalid_value_fallback() {
    let listed = load(
        "general_settings: {enforce_rpm_tpm_on_model_add: [true]}",
        &[],
    );
    assert!(
        matches!(listed, Err(ConfigError::Yaml { .. })),
        "{listed:?}"
    );
    let text = load(
        "general_settings: {enforce_rpm_tpm_on_model_add: maybe}",
        &[],
    );
    match text {
        Err(ConfigError::InvalidScalar { path, .. }) => {
            assert_eq!(path, "general_settings.enforce_rpm_tpm_on_model_add");
        }
        other => panic!("plain text is not a flag: {other:?}"),
    }
    let referenced = load(
        "general_settings: {enforce_rpm_tpm_on_model_add: os.environ/ENFORCE_LIMITS}",
        &[("ENFORCE_LIMITS", "perhaps")],
    );
    match referenced {
        Err(ConfigError::Environment { source }) => {
            assert!(source.to_string().contains("ENFORCE_LIMITS"), "{source}");
        }
        other => panic!("an unreadable variable fails the load: {other:?}"),
    }
}
