#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::file_upload_validation::Settings;

fn load(source: &str) -> Result<Settings, ConfigError> {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .map(|config| config.sections.file_upload_validation)
}

#[test]
fn loads_settings_absent_keys_take_the_defaults() {
    assert_eq!(load("{}").unwrap(), Settings::default());
}

#[test]
fn loads_settings_integer_overrides() {
    let settings =
        load("general_settings: {max_file_size_mb: 10, max_batch_file_size_mb: 20}").unwrap();
    assert_eq!(settings.max_file_size_mb, Some(10));
    assert_eq!(settings.max_batch_file_size_mb, Some(20));
}

#[test]
fn loads_settings_string_list_overrides() {
    let source = "general_settings: {allowed_file_extensions: [.pdf, .txt], blocked_file_extensions: [.exe]}";
    let settings = load(source).unwrap();
    assert_eq!(
        settings.allowed_file_extensions,
        Some(vec![".pdf".to_owned(), ".txt".to_owned()])
    );
    assert_eq!(
        settings.blocked_file_extensions,
        Some(vec![".exe".to_owned()])
    );
}

#[test]
fn loads_settings_empty_list_and_null_stay_distinct() {
    let source = "general_settings: {allowed_file_extensions: [], blocked_file_extensions: null}";
    let settings = load(source).unwrap();
    assert_eq!(settings.allowed_file_extensions, Some(Vec::new()));
    assert_eq!(settings.blocked_file_extensions, None);
}

#[test]
fn loads_settings_keys_are_known_to_the_loader() {
    let source = "general_settings: {max_file_size_mb: 1, max_batch_file_size_mb: 1, allowed_file_extensions: [.pdf], blocked_file_extensions: [.exe]}";
    assert!(load(source).is_ok());
}

#[rstest]
#[case::boolean("general_settings: {max_file_size_mb: true}")]
#[case::fractional("general_settings: {max_file_size_mb: 1.5}")]
#[case::text("general_settings: {max_file_size_mb: '1'}")]
#[case::integral_float("general_settings: {max_file_size_mb: 1.0}")]
#[case::list("general_settings: {max_file_size_mb: [1]}")]
fn loads_settings_rejects_strict_integers(#[case] source: &str) {
    assert!(matches!(
        load(source),
        Err(ConfigError::InvalidScalar { .. })
    ));
}

#[rstest]
#[case::text("general_settings: {max_batch_file_size_mb: '1'}", Some(1))]
#[case::boolean("general_settings: {max_batch_file_size_mb: true}", Some(1))]
#[case::false_boolean("general_settings: {max_batch_file_size_mb: false}", Some(0))]
#[case::integral_float("general_settings: {max_batch_file_size_mb: 1.0}", Some(1))]
#[case::negative("general_settings: {max_batch_file_size_mb: -2}", Some(-2))]
fn loads_settings_accepts_lax_batch_integers(#[case] source: &str, #[case] expected: Option<i64>) {
    assert_eq!(load(source).unwrap().max_batch_file_size_mb, expected);
}

#[rstest]
#[case::fractional("general_settings: {max_batch_file_size_mb: 1.5}")]
#[case::list("general_settings: {max_batch_file_size_mb: [1]}")]
#[case::empty_text("general_settings: {max_batch_file_size_mb: ''}")]
#[case::word("general_settings: {max_batch_file_size_mb: 'one'}")]
fn loads_settings_rejects_lax_batch_integers(#[case] source: &str) {
    assert!(matches!(
        load(source),
        Err(ConfigError::InvalidScalar { .. })
    ));
}

#[rstest]
#[case::text("general_settings: {allowed_file_extensions: '.pdf'}")]
#[case::mixed("general_settings: {allowed_file_extensions: [.pdf, 1]}")]
#[case::boolean("general_settings: {allowed_file_extensions: true}")]
fn loads_settings_rejects_extension_types(#[case] source: &str) {
    assert!(matches!(
        load(source),
        Err(ConfigError::InvalidScalar { .. })
    ));
}
