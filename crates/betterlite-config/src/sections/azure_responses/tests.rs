#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{DEFAULT_VERSION, Settings, VERSION_VARIABLE, default_responses_api_version};
use crate::loader::{ConfigError, ConfigLoader};

// the real loader runs first, so the version is read from the lookup the loaded config was built with
fn load(source: &str, environment: &[(&str, &str)]) -> Result<String, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    let loader = ConfigLoader { lookup };
    loader.load(source)?;
    Ok(default_responses_api_version(&loader.lookup))
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
            assert_eq!(configured, "2025-04-01-preview");
            assert_eq!(load("", &[]).unwrap(), DEFAULT_VERSION);
        }
        1 => {
            assert_eq!(load("model_list: []", &[]).unwrap(), "preview");
        }
        2 => {
            assert_eq!(
                load("", &[(VERSION_VARIABLE, "")]).unwrap(),
                DEFAULT_VERSION
            );
        }
        _ => {
            let error = load(
                "litellm_settings: {azure_responses_api_version: 2025-04-01-preview}",
                &[(VERSION_VARIABLE, "2025-04-01-preview")],
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

#[test]
fn settings_add_nothing_to_sections() {
    assert_eq!(size_of::<Settings>(), 0);
}
