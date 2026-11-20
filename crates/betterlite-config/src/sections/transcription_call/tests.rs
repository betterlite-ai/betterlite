#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::ExposeSecret as _;

use super::Settings;
use crate::loader::ConfigLoader;

fn settings(source: &str, variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .expect("settings load")
        .sections
        .transcription_call
}

fn secret(value: &Option<secrecy::SecretString>) -> &str {
    value.as_ref().expect("secret").expose_secret()
}

mod loads_settings {
    use super::*;

    #[test]
    fn globals() {
        let settings = settings(
            "litellm_settings:\n  api_base: https://global.example\n  api_version: '2024-06-01'\n  api_key: global-key\n  openai_key: configured-openai-key\n  azure_key: configured-azure-key\n  organization: global-org\n  drop_params: true\n",
            &[],
        );
        assert_eq!(settings.api_base.as_deref(), Some("https://global.example"));
        assert_eq!(settings.api_version.as_deref(), Some("2024-06-01"));
        assert_eq!(secret(&settings.api_key), "global-key");
        assert_eq!(secret(&settings.openai_key), "configured-openai-key");
        assert_eq!(secret(&settings.azure_key), "configured-azure-key");
        assert_eq!(settings.organization.as_deref(), Some("global-org"));
        assert_eq!(settings.drop_params, Some(true));
    }

    #[test]
    fn openai_environment() {
        let settings = settings(
            "litellm_settings: {}\n",
            &[
                ("OPENAI_BASE_URL", "https://env.example/v1"),
                ("OPENAI_API_BASE", "https://legacy.example/v1"),
                ("OPENAI_API_KEY", "env-openai-key"),
                ("OPENAI_ORGANIZATION", "env-org"),
            ],
        );
        assert_eq!(
            settings.openai_base_url.as_deref(),
            Some("https://env.example/v1")
        );
        assert_eq!(
            settings.openai_api_base.as_deref(),
            Some("https://legacy.example/v1")
        );
        assert_eq!(secret(&settings.openai_api_key), "env-openai-key");
        assert_eq!(settings.openai_organization.as_deref(), Some("env-org"));
    }

    #[test]
    fn azure_environment() {
        let settings = settings(
            "litellm_settings: {}\n",
            &[
                ("AZURE_API_BASE", "https://azure.example"),
                ("AZURE_API_VERSION", "2024-10-21"),
                ("AZURE_AD_TOKEN", "env-ad-token"),
                ("AZURE_API_KEY", "env-azure-key"),
                ("AZURE_OPENAI_API_KEY", "env-azure-openai-key"),
            ],
        );
        assert_eq!(
            settings.azure_api_base.as_deref(),
            Some("https://azure.example")
        );
        assert_eq!(settings.azure_api_version.as_deref(), Some("2024-10-21"));
        assert_eq!(secret(&settings.azure_ad_token), "env-ad-token");
        assert_eq!(secret(&settings.azure_api_key), "env-azure-key");
        assert_eq!(
            secret(&settings.azure_openai_api_key),
            "env-azure-openai-key"
        );
    }

    #[test]
    fn environment_references_resolve() {
        let settings = settings(
            "litellm_settings:\n  api_key: os.environ/GLOBAL_KEY\n  organization: os.environ/GLOBAL_ORG\n",
            &[
                ("GLOBAL_KEY", "resolved-key"),
                ("GLOBAL_ORG", "resolved-org"),
            ],
        );
        assert_eq!(secret(&settings.api_key), "resolved-key");
        assert_eq!(settings.organization.as_deref(), Some("resolved-org"));
    }

    #[test]
    fn an_empty_environment_value_stays_empty() {
        let settings = settings(
            "litellm_settings: {}\n",
            &[("AZURE_API_KEY", ""), ("OPENAI_ORGANIZATION", "")],
        );
        assert_eq!(secret(&settings.azure_api_key), "");
        assert_eq!(settings.openai_organization.as_deref(), Some(""));
    }
}

#[test]
fn the_shipped_seed_loads() {
    let source = include_str!("../../../fuzz/corpus/config_yaml/transcription-call-globals.yaml");
    let settings = settings(source, &[]);
    assert_eq!(settings.api_base.as_deref(), Some("https://seed.example"));
    assert_eq!(secret(&settings.openai_key), "seed-openai-key");
    assert_eq!(settings.drop_params, Some(false));
}
