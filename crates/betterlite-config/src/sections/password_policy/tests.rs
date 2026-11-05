#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::password_policy::{CharacterClass, Settings};

fn load(yaml: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load(yaml)?;
    Ok(config.sections.password_policy)
}

#[test]
fn loads_settings() {
    let yaml = "general_settings:
  password_policy_min_length: 16
  password_policy_require_uppercase: false
  password_policy_require_lowercase: false
  password_policy_require_numbers: false
  password_policy_require_special_characters: false
  password_policy_check_breached_passwords: false
";
    let settings = load(yaml, &[]).unwrap();
    assert_eq!(
        settings,
        Settings {
            min_length: 16,
            required: Vec::new(),
            check_breached_passwords: false,
        }
    );
}

#[rstest]
#[case::absent("general_settings: {}\n", 12)]
#[case::explicit_null("general_settings:\n  password_policy_min_length: ~\n", 12)]
#[case::above_floor("general_settings:\n  password_policy_min_length: 20\n", 20)]
#[case::below_floor("general_settings:\n  password_policy_min_length: 3\n", 8)]
#[case::zero("general_settings:\n  password_policy_min_length: 0\n", 8)]
#[case::negative("general_settings:\n  password_policy_min_length: -40\n", 8)]
#[case::whole_float("general_settings:\n  password_policy_min_length: 14.0\n", 14)]
fn defaults_and_floor(#[case] yaml: &str, #[case] min_length: u32) {
    let settings = load(yaml, &[]).unwrap();
    assert_eq!(settings.min_length, min_length);
    assert_eq!(settings.required, Settings::default().required);
    assert_eq!(settings.required.len(), 4);
    assert!(settings.check_breached_passwords);
}

#[test]
fn defaults_and_precedence() {
    assert_eq!(load("{}\n", &[]).unwrap(), Settings::default());
    let yaml = "general_settings:
  password_policy_min_length: os.environ/POLICY_LENGTH
  password_policy_require_uppercase: os.environ/POLICY_UPPER
  password_policy_require_lowercase: true
  password_policy_require_numbers: os.environ/POLICY_NUMBERS
  password_policy_require_special_characters: os.environ/POLICY_SPECIAL
  password_policy_check_breached_passwords: os.environ/POLICY_BREACH
";
    let environment = [
        ("POLICY_LENGTH", " 24 "),
        ("POLICY_UPPER", "no"),
        ("POLICY_NUMBERS", "OFF"),
        ("POLICY_SPECIAL", ""),
        ("POLICY_BREACH", "0"),
    ];
    let settings = load(yaml, &environment).unwrap();
    assert_eq!(settings.min_length, 24);
    assert_eq!(
        settings.required,
        [CharacterClass::Lowercase, CharacterClass::Special]
    );
    assert!(!settings.check_breached_passwords);
}

#[test]
fn environment_length_is_floored_and_empty_is_unset() {
    let yaml = "general_settings:\n  password_policy_min_length: os.environ/POLICY_LENGTH\n";
    assert_eq!(load(yaml, &[("POLICY_LENGTH", "2")]).unwrap().min_length, 8);
    assert_eq!(load(yaml, &[("POLICY_LENGTH", "")]).unwrap().min_length, 12);
}

#[rstest]
#[case::text_length("password_policy_min_length: twelve", &[], "password_policy_min_length")]
#[case::fractional_length("password_policy_min_length: 12.5", &[], "password_policy_min_length")]
#[case::bool_length("password_policy_min_length: true", &[], "password_policy_min_length")]
#[case::env_text_length("password_policy_min_length: os.environ/BAD", &[("BAD", "many")], "BAD")]
#[case::missing_env_length("password_policy_min_length: os.environ/GONE", &[], "GONE")]
#[case::text_flag("password_policy_require_numbers: nope", &[], "password_policy_require_numbers")]
#[case::number_flag("password_policy_require_uppercase: 0", &[], "password_policy_require_uppercase")]
#[case::env_text_flag("password_policy_check_breached_passwords: os.environ/BAD", &[("BAD", "perhaps")], "BAD")]
#[case::missing_env_flag("password_policy_require_lowercase: os.environ/GONE", &[], "GONE")]
fn invalid_value_fallback(
    #[case] line: &str,
    #[case] environment: &[(&str, &str)],
    #[case] named: &str,
) {
    let yaml = format!("general_settings:\n  {line}\n");
    let error = load(&yaml, environment).unwrap_err();
    assert!(
        error.to_string().contains(named),
        "{error} should name {named}"
    );
}
