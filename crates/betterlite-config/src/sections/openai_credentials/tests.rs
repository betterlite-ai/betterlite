#![cfg(test)]

use crate::loader::{ConfigError, ConfigLoader};
use rstest::rstest;
use std::collections::BTreeMap;

fn load(source: &str, environment: &[(&str, &str)]) -> Result<super::Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .openai_credentials)
}

#[rstest]
#[case::loads_openai_base_url(0)]
#[case::loads_openai_api_base(1)]
#[case::loads_openai_organization(2)]
#[case::rejects_unrelated_key(3)]
fn openai_credentials_config_cases(#[case] case: u8) {
    match case {
        0 => {
            let default = load("", &[]).unwrap();
            assert_eq!(default, super::Settings::default());
            assert_eq!(default.api_base, super::DEFAULT_API_BASE);
            let from_base_url = load("", &[("OPENAI_BASE_URL", "https://base.example/v1")])
                .unwrap()
                .api_base;
            assert_eq!(from_base_url, "https://base.example/v1");
            let precedence = load(
                "",
                &[
                    ("OPENAI_BASE_URL", "https://base.example/v1"),
                    ("OPENAI_API_BASE", "https://api-base.example/v1"),
                ],
            )
            .unwrap()
            .api_base;
            assert_eq!(precedence, "https://base.example/v1");
        }
        1 => {
            let from_api_base = load("", &[("OPENAI_API_BASE", "https://api-base.example/v1")])
                .unwrap()
                .api_base;
            assert_eq!(from_api_base, "https://api-base.example/v1");
            let empty_falls_back_to_default =
                load("", &[("OPENAI_API_BASE", "")]).unwrap().api_base;
            assert_eq!(empty_falls_back_to_default, super::DEFAULT_API_BASE);
        }
        2 => {
            let default = load("", &[]).unwrap();
            assert!(default.organization.is_none());
            let from_env = load("", &[("OPENAI_ORGANIZATION", "org-123")])
                .unwrap()
                .organization;
            assert_eq!(from_env, Some("org-123".to_owned()));
            let empty_falls_through = load("", &[("OPENAI_ORGANIZATION", "")])
                .unwrap()
                .organization;
            assert!(empty_falls_through.is_none());
        }
        _ => {
            let source = "litellm_settings:\n  openai_credentials_unrelated_key: 1\n";
            assert!(matches!(
                load(source, &[]),
                Err(ConfigError::Unsupported { .. })
            ));
        }
    }
}
