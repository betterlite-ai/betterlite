#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::{API_VERSION_VAR, Settings};
use crate::loader::{ConfigError, ConfigLoader};
use crate::types::Config;

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    Ok(load_config(source, environment)?
        .sections
        .foundry_image_edits)
}

fn load_config(source: &str, environment: &[(&str, &str)]) -> Result<Config, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }.load(source)
}

#[rstest]
#[case::loads_azure_ai_api_version(0)]
#[case::rejects_unrelated_key(1)]
fn foundry_image_edits_config_cases(#[case] case: u8) {
    if case == 0 {
        assert_eq!(API_VERSION_VAR, "AZURE_AI_API_VERSION");
        let absent = load("", &[]).unwrap();
        assert!(absent.api_version.is_none());
        let from_env = load("", &[("AZURE_AI_API_VERSION", "2025-11-15")]).unwrap();
        assert_eq!(from_env.api_version.as_deref(), Some("2025-11-15"));
        // an empty environment value counts as unset, so the variant's own default applies
        let empty = load("", &[("AZURE_AI_API_VERSION", "")]).unwrap();
        assert!(empty.api_version.is_none());
        assert_eq!(
            format!("{from_env:?}"),
            "Settings { api_version: Some(\"2025-11-15\") }"
        );
        let config = load_config(
            "litellm_settings:\n  api_version: 2024-02-01\n",
            &[("AZURE_AI_API_VERSION", "2025-11-15")],
        )
        .unwrap();
        assert_eq!(
            config.sections.foundry_image_edits.api_version.as_deref(),
            Some("2025-11-15")
        );
        assert_eq!(
            config.sections.resource_sdk_settings.api_version.as_deref(),
            Some("2024-02-01")
        );
    } else {
        let source = "litellm_settings:\n  foundry_image_edits_api_version: 2025-11-15\n";
        let error = load(source, &[]).unwrap_err();
        assert_eq!(error.category(), "unsupported_key");
        let ConfigError::Unsupported { paths } = error else {
            panic!("unexpected error: {error:?}");
        };
        assert_eq!(
            paths,
            vec!["litellm_settings.foundry_image_edits_api_version".to_owned()]
        );
    }
}
