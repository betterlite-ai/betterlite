#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::{DEFAULT_API_BASE, Settings};
use crate::loader::{ConfigError, ConfigLoader};

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .anthropic_credentials)
}

fn revealed(value: Option<&SecretString>) -> Option<&str> {
    value.map(ExposeSecret::expose_secret)
}

#[rstest]
#[case::loads_anthropic_api_key(0)]
#[case::loads_anthropic_auth_token(1)]
#[case::loads_anthropic_api_base(2)]
#[case::loads_anthropic_base_url(3)]
#[case::loads_use_bearer_for_custom_base(4)]
#[case::loads_litellm_anthropic_disable_url_suffix(5)]
#[case::rejects_unrelated_key(6)]
fn anthropic_credentials_config_cases(#[case] case: u8) {
    match case {
        0 => {
            let default = load("", &[]).unwrap();
            assert!(default.api_key.is_none());
            let from_env = load("", &[("ANTHROPIC_API_KEY", "sk-ant-key")]).unwrap();
            assert_eq!(revealed(from_env.api_key.as_ref()), Some("sk-ant-key"));
            let empty = load("", &[("ANTHROPIC_API_KEY", "")]).unwrap();
            assert_eq!(revealed(empty.api_key.as_ref()), Some(""));
            assert!(!format!("{from_env:?}").contains("sk-ant-key"));
        }
        1 => {
            let default = load("", &[]).unwrap();
            assert!(default.auth_token.is_none());
            let from_env = load("", &[("ANTHROPIC_AUTH_TOKEN", "auth-token-value")]).unwrap();
            assert_eq!(
                revealed(from_env.auth_token.as_ref()),
                Some("auth-token-value")
            );
            let empty = load("", &[("ANTHROPIC_AUTH_TOKEN", "")]).unwrap();
            assert_eq!(revealed(empty.auth_token.as_ref()), Some(""));
            assert!(!format!("{from_env:?}").contains("auth-token-value"));
        }
        2 => {
            assert_eq!(DEFAULT_API_BASE, "https://api.anthropic.com");
            assert_eq!(load("", &[]).unwrap().api_base, DEFAULT_API_BASE);
            let from_api_base = load("", &[("ANTHROPIC_API_BASE", "https://api-base.example")])
                .unwrap()
                .api_base;
            assert_eq!(from_api_base, "https://api-base.example");
            let precedence = load(
                "",
                &[
                    ("ANTHROPIC_API_BASE", "https://api-base.example"),
                    ("ANTHROPIC_BASE_URL", "https://base-url.example"),
                ],
            )
            .unwrap()
            .api_base;
            assert_eq!(precedence, "https://api-base.example");
            let empty_falls_through = load(
                "",
                &[
                    ("ANTHROPIC_API_BASE", ""),
                    ("ANTHROPIC_BASE_URL", "https://base-url.example"),
                ],
            )
            .unwrap()
            .api_base;
            assert_eq!(empty_falls_through, "https://base-url.example");
        }
        3 => {
            let from_base_url = load("", &[("ANTHROPIC_BASE_URL", "https://base-url.example")])
                .unwrap()
                .api_base;
            assert_eq!(from_base_url, "https://base-url.example");
            let empty = load("", &[("ANTHROPIC_BASE_URL", "")]).unwrap().api_base;
            assert_eq!(empty, DEFAULT_API_BASE);
        }
        4 => {
            let default = load("", &[]).unwrap();
            assert!(!default.use_bearer_for_custom_base);
            assert!(default.deployment_bearer_opt_ins.is_empty());
            let module = load(
                "litellm_settings:\n  use_bearer_for_custom_base: true\n",
                &[],
            )
            .unwrap();
            assert!(module.use_bearer_for_custom_base);
            let enabled = "model_list:\n  - model_name: claude\n    litellm_params:\n      model: anthropic/claude-sonnet-4\n      use_bearer_for_custom_base: true\n";
            assert_eq!(
                load(enabled, &[]).unwrap().deployment_bearer_opt_ins,
                vec![true]
            );
            let disabled = "model_list:\n  - model_name: claude\n    litellm_params:\n      model: anthropic/claude-sonnet-4\n      use_bearer_for_custom_base: false\n";
            assert_eq!(
                load(
                    "litellm_settings:\n  use_bearer_for_custom_base: true\n",
                    &[]
                )
                .unwrap()
                .deployment_bearer_opt_ins,
                Vec::<bool>::new()
            );
            let inherited = load(
                "litellm_settings:\n  use_bearer_for_custom_base: true\nmodel_list:\n  - model_name: first\n  - model_name: second\n",
                &[],
            )
            .unwrap();
            assert_eq!(inherited.deployment_bearer_opt_ins, vec![true, true]);
            let overridden = load(
                "litellm_settings:\n  use_bearer_for_custom_base: true\nmodel_list:\n  - model_name: first\n    litellm_params:\n      use_bearer_for_custom_base: false\n  - model_name: second\n",
                &[],
            )
            .unwrap();
            assert_eq!(overridden.deployment_bearer_opt_ins, vec![false, true]);
            assert_eq!(
                load(disabled, &[]).unwrap().deployment_bearer_opt_ins,
                vec![false],
                "an explicit deployment value stays false"
            );
        }
        5 => {
            let read = |value: &str| load("", &[("LITELLM_ANTHROPIC_DISABLE_URL_SUFFIX", value)]);
            assert!(!load("", &[]).unwrap().disable_url_suffix);
            for value in ["true", "TRUE", "1", "yes", "on", " true "] {
                assert!(read(value).unwrap().disable_url_suffix, "{value}");
            }
            for value in ["false", "0", "no", "off", ""] {
                assert!(!read(value).unwrap().disable_url_suffix, "{value}");
            }
            assert!(matches!(
                read("maybe"),
                Err(ConfigError::Environment { .. })
            ));
        }
        _ => {
            let source = "litellm_settings:\n  anthropic_credentials_unrelated_key: 1\n";
            assert!(matches!(
                load(source, &[]),
                Err(ConfigError::Unsupported { .. })
            ));
        }
    }
}
