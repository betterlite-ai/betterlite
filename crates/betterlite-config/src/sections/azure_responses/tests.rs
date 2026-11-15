#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{DEFAULT_VERSION, Settings, VERSION_VARIABLE};
use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .azure_responses)
}

#[rstest]
#[case::loads_azure_default_responses_api_version(0)]
#[case::defaults_when_the_variable_is_absent(1)]
#[case::empty_variable_counts_as_unset(2)]
#[case::rejects_unrelated_key(3)]
fn azure_responses_config_cases(#[case] case: u8) {
    match case {
        0 => {
            let configured = load("", &[(VERSION_VARIABLE, "2025-04-01-preview")]).unwrap();
            assert_eq!(
                configured.default_responses_api_version,
                "2025-04-01-preview"
            );
            let default = load("", &[]).unwrap();
            assert_eq!(default.default_responses_api_version, DEFAULT_VERSION);
            assert_eq!(Settings::default().default_responses_api_version, "preview");
        }
        1 => {
            let loaded = load("model_list: []", &[]).unwrap();
            assert_eq!(loaded.default_responses_api_version, "preview");
        }
        2 => {
            let loaded = load("", &[(VERSION_VARIABLE, "")]).unwrap();
            assert_eq!(loaded.default_responses_api_version, DEFAULT_VERSION);
        }
        _ => {
            let error = load(
                "litellm_settings: {azure_responses_api_version: 2025-04-01-preview}",
                &[],
            )
            .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("litellm_settings.azure_responses_api_version")
            );
        }
    }
}
