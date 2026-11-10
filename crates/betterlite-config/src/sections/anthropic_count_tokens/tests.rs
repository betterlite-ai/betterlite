#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{BETA_VERSION_VARIABLE, DEFAULT_BETA_VERSION, Settings};
use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .anthropic_count_tokens)
}

#[rstest]
#[case::loads_anthropic_token_counting_beta_version(0)]
#[case::defaults_when_the_variable_is_absent(1)]
#[case::empty_variable_counts_as_unset(2)]
#[case::rejects_unrelated_key(3)]
fn anthropic_count_config_cases(#[case] case: u8) {
    match case {
        0 => {
            let default = load("", &[]).unwrap();
            assert_eq!(default.beta_version, DEFAULT_BETA_VERSION);
            let configured =
                load("", &[(BETA_VERSION_VARIABLE, "token-counting-2025-06-01")]).unwrap();
            assert_eq!(configured.beta_version, "token-counting-2025-06-01");
            assert_eq!(Settings::default().beta_version, DEFAULT_BETA_VERSION);
        }
        1 => {
            let loaded = load("model_list: []", &[]).unwrap();
            assert_eq!(loaded.beta_version, "token-counting-2024-11-01");
        }
        2 => {
            let loaded = load("", &[(BETA_VERSION_VARIABLE, "")]).unwrap();
            assert_eq!(loaded.beta_version, DEFAULT_BETA_VERSION);
        }
        _ => {
            let error = load(
                "litellm_settings: {anthropic_token_counting_beta_version: token-counting-2025-06-01}",
                &[],
            )
            .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("litellm_settings.anthropic_token_counting_beta_version")
            );
        }
    }
}
