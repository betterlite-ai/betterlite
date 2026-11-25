#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::Settings;

const EMPTY: &str = "general_settings: {}\n";

#[derive(Clone, Copy)]
enum Expected {
    Disable(Option<bool>),
    Providers(Option<&'static [&'static str]>),
    Models(Option<&'static [&'static str]>),
    InvalidScalar,
}

fn load(yaml: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(yaml)
        .map(|config| config.sections.batch_limit_descriptors)
}

fn owned(names: Option<&[&str]>) -> Option<Vec<String>> {
    let entries = names?;
    Some(entries.iter().map(|name| (*name).to_owned()).collect())
}

#[rstest]
#[case::disable_explicit_true("general_settings:\n  disable_batch_input_file_rate_limiting: true\n", &[], Expected::Disable(Some(true)))]
#[case::disable_explicit_false("general_settings:\n  disable_batch_input_file_rate_limiting: false\n", &[], Expected::Disable(Some(false)))]
#[case::disable_unset(EMPTY, &[], Expected::Disable(None))]
#[case::disable_null("general_settings:\n  disable_batch_input_file_rate_limiting: null\n", &[], Expected::Disable(None))]
#[case::disable_invalid("general_settings:\n  disable_batch_input_file_rate_limiting: maybe\n", &[], Expected::InvalidScalar)]
#[case::disable_invalid_number("general_settings:\n  disable_batch_input_file_rate_limiting: 1\n", &[], Expected::InvalidScalar)]
#[case::disable_environment("general_settings:\n  disable_batch_input_file_rate_limiting: os.environ/BATCH_DISABLE\n", &[("BATCH_DISABLE", "true")], Expected::Disable(Some(true)))]
#[case::disable_empty_environment("general_settings:\n  disable_batch_input_file_rate_limiting: os.environ/BATCH_DISABLE\n", &[("BATCH_DISABLE", "")], Expected::Disable(None))]
#[case::providers_explicit("general_settings:\n  skip_batch_input_file_rate_limiting_for_providers:\n    - openai\n    - azure\n", &[], Expected::Providers(Some(&["openai", "azure"])))]
#[case::providers_empty("general_settings:\n  skip_batch_input_file_rate_limiting_for_providers: []\n", &[], Expected::Providers(Some(&[])))]
#[case::providers_unset(EMPTY, &[], Expected::Providers(None))]
#[case::providers_null("general_settings:\n  skip_batch_input_file_rate_limiting_for_providers: null\n", &[], Expected::Providers(None))]
#[case::providers_invalid("general_settings:\n  skip_batch_input_file_rate_limiting_for_providers: openai\n", &[], Expected::InvalidScalar)]
#[case::providers_invalid_item("general_settings:\n  skip_batch_input_file_rate_limiting_for_providers:\n    - openai\n    - 3\n", &[], Expected::InvalidScalar)]
#[case::providers_environment("general_settings:\n  skip_batch_input_file_rate_limiting_for_providers:\n    - os.environ/SKIP_PROVIDER\n", &[("SKIP_PROVIDER", "azure")], Expected::Providers(Some(&["azure"])))]
#[case::models_explicit("general_settings:\n  skip_batch_input_file_rate_limiting_for_models:\n    - gpt-4o\n", &[], Expected::Models(Some(&["gpt-4o"])))]
#[case::models_empty("general_settings:\n  skip_batch_input_file_rate_limiting_for_models: []\n", &[], Expected::Models(Some(&[])))]
#[case::models_unset(EMPTY, &[], Expected::Models(None))]
#[case::models_null("general_settings:\n  skip_batch_input_file_rate_limiting_for_models: null\n", &[], Expected::Models(None))]
#[case::models_invalid("general_settings:\n  skip_batch_input_file_rate_limiting_for_models:\n    gpt: 1\n", &[], Expected::InvalidScalar)]
#[case::all_keys_accepted(
    "general_settings:\n  disable_batch_input_file_rate_limiting: true\n  skip_batch_input_file_rate_limiting_for_providers:\n    - openai\n  skip_batch_input_file_rate_limiting_for_models:\n    - gpt-4o\n",
    &[],
    Expected::Disable(Some(true))
)]
fn loads_settings(
    #[case] yaml: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: Expected,
) {
    match expected {
        Expected::Disable(wanted) => {
            assert_eq!(
                load(yaml, environment)
                    .expect("settings load")
                    .disable_rate_limiting,
                wanted
            );
        }
        Expected::Providers(wanted) => {
            assert_eq!(
                load(yaml, environment)
                    .expect("settings load")
                    .skip_providers,
                owned(wanted)
            );
        }
        Expected::Models(wanted) => {
            assert_eq!(
                load(yaml, environment).expect("settings load").skip_models,
                owned(wanted)
            );
        }
        Expected::InvalidScalar => {
            assert_eq!(
                load(yaml, environment).unwrap_err().category(),
                "invalid_scalar"
            );
        }
    }
}

#[test]
fn all_keys_load_together() {
    let settings = load(
        "general_settings:\n  disable_batch_input_file_rate_limiting: false\n  skip_batch_input_file_rate_limiting_for_providers: [openai]\n  skip_batch_input_file_rate_limiting_for_models: [gpt-4o]\n",
        &[],
    )
    .expect("settings load");
    assert_eq!(settings.disable_rate_limiting, Some(false));
    assert_eq!(settings.skip_providers, Some(vec!["openai".to_owned()]));
    assert_eq!(settings.skip_models, Some(vec!["gpt-4o".to_owned()]));
}
