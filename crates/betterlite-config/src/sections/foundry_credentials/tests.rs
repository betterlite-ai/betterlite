#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::{API_BASE_VAR, API_KEY_VAR, Settings};
use crate::loader::{ConfigError, ConfigLoader};
use crate::types::Config;

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let config = load_config(source, environment)?;
    Ok(config.sections.foundry_credentials)
}

fn load_config(source: &str, environment: &[(&str, &str)]) -> Result<Config, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }.load(source)
}

fn revealed(value: Option<&SecretString>) -> Option<&str> {
    value.map(ExposeSecret::expose_secret)
}

#[rstest]
#[case::loads_azure_ai_api_base(0)]
#[case::loads_azure_ai_api_key(1)]
#[case::rejects_unrelated_key(2)]
fn foundry_credentials_config_cases(#[case] case: u8) {
    match case {
        0 => {
            assert_eq!(API_BASE_VAR, "AZURE_AI_API_BASE");
            let absent = load("", &[]).unwrap();
            assert!(absent.api_base.is_none());
            let from_env = load("", &[("AZURE_AI_API_BASE", "https://env.example")]).unwrap();
            assert_eq!(from_env.api_base.as_deref(), Some("https://env.example"));
            let empty = load("", &[("AZURE_AI_API_BASE", "")]).unwrap();
            assert_eq!(empty.api_base.as_deref(), Some(""));
            let config = load_config(
                "litellm_settings:\n  api_base: https://global.example\n",
                &[("AZURE_AI_API_BASE", "https://env.example")],
            )
            .unwrap();
            assert_eq!(
                config.sections.foundry_credentials.api_base.as_deref(),
                Some("https://env.example")
            );
            assert_eq!(
                config.sections.resource_sdk_settings.api_base.as_deref(),
                Some("https://global.example")
            );
        }
        1 => {
            assert_eq!(API_KEY_VAR, "AZURE_AI_API_KEY");
            let absent = load("", &[]).unwrap();
            assert!(absent.api_key.is_none());
            let from_env = load("", &[("AZURE_AI_API_KEY", "azure-key")]).unwrap();
            assert_eq!(revealed(from_env.api_key.as_ref()), Some("azure-key"));
            assert!(!format!("{from_env:?}").contains("azure-key"));
            let empty = load("", &[("AZURE_AI_API_KEY", "")]).unwrap();
            assert_eq!(revealed(empty.api_key.as_ref()), Some(""));
            let config = load_config(
                "litellm_settings:\n  api_key: global-key\n",
                &[("AZURE_AI_API_KEY", "azure-key")],
            )
            .unwrap();
            assert_eq!(
                revealed(config.sections.foundry_credentials.api_key.as_ref()),
                Some("azure-key")
            );
            assert_eq!(
                revealed(config.sections.resource_sdk_settings.api_key.as_ref()),
                Some("global-key")
            );
            assert!(!format!("{:?}", config.sections.foundry_credentials).contains("azure-key"));
        }
        _ => {
            let source = "litellm_settings:\n  foundry_credentials_unrelated_key: 1\n";
            let error = load(source, &[]).unwrap_err();
            assert_eq!(error.category(), "unsupported_key");
            let ConfigError::Unsupported { paths } = error else {
                panic!("unexpected error: {error:?}");
            };
            assert_eq!(
                paths,
                vec!["litellm_settings.foundry_credentials_unrelated_key".to_owned()]
            );
        }
    }
}
