#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::team_fields::{
    DEFAULT_TIMEOUT, DEFAULT_UNAVAILABLE_MESSAGE, MetadataSchemaField, Settings, ValidationTimeout,
};

fn loaded_with(source: &str, lookup: BTreeMap<String, String>) -> Settings {
    let config = ConfigLoader { lookup }.load(source).unwrap();
    config.sections.team_fields
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

#[test]
fn metadata_settings_loaded() {
    let source = "\
general_settings:
  custom_team_metadata_validate: my_module.validate
  team_metadata_schema:
    - key: cost_center
      label: Cost center
    - key: owner
  team_metadata_validation_timeout: 2.5
  team_metadata_validation_error_message: Nope.
";
    let settings = loaded(source);
    assert_eq!(settings.validator.as_deref(), Some("my_module.validate"));
    assert_eq!(
        settings.schema,
        vec![
            MetadataSchemaField {
                key: "cost_center".to_owned(),
                label: Some("Cost center".to_owned()),
            },
            MetadataSchemaField {
                key: "owner".to_owned(),
                label: None,
            },
        ]
    );
    assert_eq!(
        settings.timeout,
        ValidationTimeout::Finite(Duration::from_millis(2500))
    );
    assert_eq!(settings.unavailable_message, "Nope.");
}

#[test]
fn absent_settings_take_pinned_defaults() {
    assert_eq!(loaded("general_settings: {}"), Settings::default());
    assert_eq!(
        Settings::default().unavailable_message,
        DEFAULT_UNAVAILABLE_MESSAGE
    );
    assert_eq!(
        Settings::default().timeout,
        ValidationTimeout::Finite(DEFAULT_TIMEOUT)
    );
    assert_eq!(DEFAULT_TIMEOUT, Duration::from_secs(5));
}

#[rstest]
#[case::integer("5", ValidationTimeout::Finite(Duration::from_secs(5)))]
#[case::float("0.25", ValidationTimeout::Finite(Duration::from_millis(250)))]
#[case::large_integer("3600", ValidationTimeout::Finite(Duration::from_secs(3600)))]
fn timeout_positive(#[case] raw: &str, #[case] expected: ValidationTimeout) {
    let source = format!("general_settings:\n  team_metadata_validation_timeout: {raw}\n");
    assert_eq!(loaded(&source).timeout, expected);
}

#[test]
fn timeout_positive_from_environment() {
    let lookup = BTreeMap::from([("TIMEOUT".to_owned(), "7".to_owned())]);
    let source = "general_settings:\n  team_metadata_validation_timeout: os.environ/TIMEOUT\n";
    assert_eq!(
        loaded_with(source, lookup).timeout,
        ValidationTimeout::Finite(Duration::from_secs(7))
    );
}

#[rstest]
#[case::string("\"5\"")]
#[case::flag("true")]
#[case::list("[5]")]
#[case::mapping("{seconds: 5}")]
fn timeout_wrong_type(#[case] raw: &str) {
    let source = format!("general_settings:\n  team_metadata_validation_timeout: {raw}\n");
    assert_eq!(
        loaded(&source).timeout,
        ValidationTimeout::Finite(DEFAULT_TIMEOUT)
    );
}

#[test]
fn timeout_wrong_type_in_environment_is_a_load_error() {
    let lookup = BTreeMap::from([("TIMEOUT".to_owned(), "soon".to_owned())]);
    let source = "general_settings:\n  team_metadata_validation_timeout: os.environ/TIMEOUT\n";
    let error = failed_with(source, lookup);
    assert_eq!(error.category(), "environment");
    assert!(error.to_string().contains("TIMEOUT"), "{error}");
}

#[rstest]
#[case::zero("0")]
#[case::float_zero("0.0")]
#[case::negative("-1")]
#[case::negative_float("-0.5")]
fn timeout_nonpositive(#[case] raw: &str) {
    let source = format!("general_settings:\n  team_metadata_validation_timeout: {raw}\n");
    assert_eq!(
        loaded(&source).timeout,
        ValidationTimeout::Finite(DEFAULT_TIMEOUT)
    );
}

#[rstest]
#[case::nan(".nan", ValidationTimeout::Finite(DEFAULT_TIMEOUT))]
#[case::infinity(".inf", ValidationTimeout::Unlimited)]
#[case::negative_infinity("-.inf", ValidationTimeout::Finite(DEFAULT_TIMEOUT))]
fn timeout_nonfinite(#[case] raw: &str, #[case] expected: ValidationTimeout) {
    let source = format!("general_settings:\n  team_metadata_validation_timeout: {raw}\n");
    assert_eq!(loaded(&source).timeout, expected);
}

#[test]
fn unavailable_message_spaces() {
    let source =
        "general_settings:\n  team_metadata_validation_error_message: \"  spaced  out  \"\n";
    assert_eq!(loaded(source).unavailable_message, "  spaced  out  ");
    let lookup = BTreeMap::from([("MESSAGE".to_owned(), "  from env  ".to_owned())]);
    let from_env =
        "general_settings:\n  team_metadata_validation_error_message: os.environ/MESSAGE\n";
    assert_eq!(
        loaded_with(from_env, lookup).unavailable_message,
        "  from env  "
    );
}

#[rstest]
#[case::absent("general_settings: {}")]
#[case::explicit_null("general_settings:\n  team_metadata_validation_error_message: null")]
#[case::empty("general_settings:\n  team_metadata_validation_error_message: \"\"")]
#[case::blank("general_settings:\n  team_metadata_validation_error_message: \"   \"")]
#[case::number("general_settings:\n  team_metadata_validation_error_message: 42")]
#[case::flag("general_settings:\n  team_metadata_validation_error_message: true")]
fn unavailable_message_default(#[case] source: &str) {
    assert_eq!(
        loaded(source).unavailable_message,
        DEFAULT_UNAVAILABLE_MESSAGE
    );
}

#[test]
fn unavailable_message_empty_environment_is_the_default() {
    let lookup = BTreeMap::from([("MESSAGE".to_owned(), String::new())]);
    let source =
        "general_settings:\n  team_metadata_validation_error_message: os.environ/MESSAGE\n";
    assert_eq!(
        loaded_with(source, lookup).unavailable_message,
        DEFAULT_UNAVAILABLE_MESSAGE
    );
}

#[rstest]
#[case::absent("general_settings: {}")]
#[case::explicit_null("general_settings:\n  team_metadata_schema: null")]
#[case::empty_list("general_settings:\n  team_metadata_schema: []")]
fn schema_missing(#[case] source: &str) {
    assert!(loaded(source).schema.is_empty());
}

#[rstest]
#[case::mapping(
    "general_settings:\n  team_metadata_schema:\n    key: cost\n",
    "must be a list"
)]
#[case::string("general_settings:\n  team_metadata_schema: cost\n", "must be a list")]
#[case::scalar_entry(
    "general_settings:\n  team_metadata_schema:\n    - cost\n",
    "team_metadata_schema[0]: Input should be an object"
)]
#[case::missing_key(
    "general_settings:\n  team_metadata_schema:\n    - label: Cost\n",
    "team_metadata_schema[0].key: Field required"
)]
#[case::empty_key(
    "general_settings:\n  team_metadata_schema:\n    - key: \"\"\n",
    "team_metadata_schema[0].key: String should have at least 1 character"
)]
#[case::numeric_key(
    "general_settings:\n  team_metadata_schema:\n    - key: 3\n",
    "team_metadata_schema[0].key: Input should be a valid string"
)]
#[case::numeric_label(
    "general_settings:\n  team_metadata_schema:\n    - key: cost\n      label: 3\n",
    "team_metadata_schema[0].label: Input should be a valid string"
)]
#[case::extra_field(
    "general_settings:\n  team_metadata_schema:\n    - key: cost\n      extra: true\n",
    "team_metadata_schema[0]: Extra inputs are not permitted"
)]
fn schema_invalid(#[case] source: &str, #[case] fragment: &str) {
    let message = yaml_message(failed(source));
    assert!(
        message.contains("general_settings.team_metadata_schema"),
        "{message}"
    );
    assert!(message.contains(fragment), "{message}");
}

#[test]
fn schema_duplicates() {
    let source = "\
general_settings:
  team_metadata_schema:
    - key: b
    - key: a
    - key: b
    - key: a
    - key: c
";
    let message = yaml_message(failed(source));
    assert!(
        message.contains("team_metadata_schema contains duplicate keys: a, b"),
        "{message}"
    );
}

#[test]
fn callable_resolution() {
    let dotted = "general_settings:\n  custom_team_metadata_validate: package.module.validator\n";
    assert_eq!(
        loaded(dotted).validator.as_deref(),
        Some("package.module.validator")
    );
    let absent = loaded("general_settings: {}");
    assert_eq!(absent.validator, None);
    let null = loaded("general_settings:\n  custom_team_metadata_validate: null\n");
    assert_eq!(null.validator, None);
    let lookup = BTreeMap::from([(
        "VALIDATOR".to_owned(),
        "env_module.env_validator".to_owned(),
    )]);
    let from_env = "general_settings:\n  custom_team_metadata_validate: os.environ/VALIDATOR\n";
    assert_eq!(
        loaded_with(from_env, lookup).validator.as_deref(),
        Some("env_module.env_validator")
    );
}

#[rstest]
#[case::number("5")]
#[case::flag("true")]
#[case::list("[validator]")]
fn callable_resolution_rejects_non_strings(#[case] raw: &str) {
    let source = format!("general_settings:\n  custom_team_metadata_validate: {raw}\n");
    let message = yaml_message(failed(&source));
    assert!(
        message.contains("general_settings.custom_team_metadata_validate"),
        "{message}"
    );
}

#[test]
fn callable_resolution_rejects_empty_name() {
    let source = "general_settings:\n  custom_team_metadata_validate: \"\"\n";
    assert_eq!(failed(source).category(), "yaml");
}

#[test]
fn callable_resolution_missing_environment() {
    let source = "general_settings:\n  custom_team_metadata_validate: os.environ/ABSENT\n";
    let error = failed(source);
    assert_eq!(error.category(), "missing_environment");
    assert!(error.to_string().contains("ABSENT"), "{error}");
}
