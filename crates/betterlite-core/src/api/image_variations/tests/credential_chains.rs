#![cfg(test)]

use betterlite_models::identity::provider_id::ProviderId;
use secrecy::{ExposeSecret as _, SecretString};

use super::super::credentials::{
    DEFAULT_OPENAI_API_BASE, DEFAULT_TOPAZ_API_BASE, VariationPolicies,
};
use super::support::{SOURCE, settings};

#[test]
fn openai_key_prefers_the_supplied_value() {
    let policies = VariationPolicies::default();
    let all = settings(
        "litellm_settings:\n  api_key: yaml-key\n  openai_key: yaml-openai-key\n",
        &[("OPENAI_API_KEY", "env-key")],
    );
    let supplied = SecretString::from("supplied");
    let supplied_key = (policies.credentials)(ProviderId::Openai, Some(&supplied), None, &all)
        .api_key
        .expect("supplied key");
    assert_eq!(supplied_key.expose_secret(), "supplied");
    let global_key = (policies.credentials)(ProviderId::Openai, None, None, &all)
        .api_key
        .expect("global key");
    assert_eq!(global_key.expose_secret(), "yaml-key");
}

#[test]
fn openai_key_falls_through_its_sources() {
    let policies = VariationPolicies::default();
    let openai_key_only = settings(
        "litellm_settings:\n  openai_key: yaml-openai-key\n",
        &[("OPENAI_API_KEY", "env-key")],
    );
    let configured_key = (policies.credentials)(ProviderId::Openai, None, None, &openai_key_only)
        .api_key
        .expect("openai_key");
    assert_eq!(configured_key.expose_secret(), "yaml-openai-key");
    let environment_only = settings(SOURCE, &[("OPENAI_API_KEY", "env-key")]);
    let environment_key = (policies.credentials)(ProviderId::Openai, None, None, &environment_only)
        .api_key
        .expect("environment key");
    assert_eq!(environment_key.expose_secret(), "env-key");
    let empty = SecretString::from("");
    let fell_through =
        (policies.credentials)(ProviderId::Openai, Some(&empty), None, &environment_only)
            .api_key
            .expect("an empty key falls through");
    assert_eq!(fell_through.expose_secret(), "env-key");
    let none = settings(SOURCE, &[]);
    assert!(
        (policies.credentials)(ProviderId::Openai, None, None, &none)
            .api_key
            .is_none()
    );
}

#[test]
fn openai_base_prefers_the_supplied_then_yaml_value() {
    let policies = VariationPolicies::default();
    let all = settings(
        "litellm_settings:\n  api_base: https://yaml.example/v1\n",
        &[("OPENAI_BASE_URL", "https://env.example/v1")],
    );
    let supplied = (policies.credentials)(
        ProviderId::Openai,
        None,
        Some("https://call.example/v1"),
        &all,
    )
    .api_base;
    assert_eq!(supplied.as_deref(), Some("https://call.example/v1"));
    let yaml = (policies.credentials)(ProviderId::Openai, None, None, &all).api_base;
    assert_eq!(yaml.as_deref(), Some("https://yaml.example/v1"));
}

#[test]
fn openai_base_falls_through_environment_and_default() {
    let policies = VariationPolicies::default();
    let environment_only = settings(
        SOURCE,
        &[
            ("OPENAI_BASE_URL", "https://env.example/v1"),
            ("OPENAI_API_BASE", "https://legacy.example/v1"),
        ],
    );
    let environment =
        (policies.credentials)(ProviderId::Openai, None, None, &environment_only).api_base;
    assert_eq!(environment.as_deref(), Some("https://env.example/v1"));
    let legacy_only = settings(SOURCE, &[("OPENAI_API_BASE", "https://legacy.example/v1")]);
    let legacy = (policies.credentials)(ProviderId::Openai, None, None, &legacy_only).api_base;
    assert_eq!(legacy.as_deref(), Some("https://legacy.example/v1"));
    let none = settings(SOURCE, &[]);
    let default_base = (policies.credentials)(ProviderId::Openai, None, None, &none).api_base;
    assert_eq!(default_base.as_deref(), Some(DEFAULT_OPENAI_API_BASE));
    let yaml = settings(
        "litellm_settings:\n  api_base: https://yaml.example/v1\n",
        &[],
    );
    let empty = (policies.credentials)(ProviderId::Openai, None, Some(""), &yaml).api_base;
    assert_eq!(empty.as_deref(), Some("https://yaml.example/v1"));
}

#[test]
fn topaz_credentials_fall_through_their_sources() {
    let policies = VariationPolicies::default();
    let topaz_environment = settings(SOURCE, &[("TOPAZ_API_KEY", "topaz-env-key")]);
    let environment_key = (policies.credentials)(ProviderId::Topaz, None, None, &topaz_environment)
        .api_key
        .expect("topaz key");
    assert_eq!(environment_key.expose_secret(), "topaz-env-key");
    let default_base =
        (policies.credentials)(ProviderId::Topaz, None, None, &topaz_environment).api_base;
    assert_eq!(default_base.as_deref(), Some(DEFAULT_TOPAZ_API_BASE));
    let supplied = SecretString::from("supplied-topaz");
    let supplied_key =
        (policies.credentials)(ProviderId::Topaz, Some(&supplied), None, &topaz_environment)
            .api_key
            .expect("supplied topaz key");
    assert_eq!(supplied_key.expose_secret(), "supplied-topaz");
    let supplied_base = (policies.credentials)(
        ProviderId::Topaz,
        None,
        Some("https://topaz.example"),
        &topaz_environment,
    )
    .api_base;
    assert_eq!(supplied_base.as_deref(), Some("https://topaz.example"));
    let with_base = settings(SOURCE, &[("TOPAZ_API_BASE", "https://topaz.example")]);
    let configured_base =
        (policies.credentials)(ProviderId::Topaz, None, None, &with_base).api_base;
    assert_eq!(configured_base.as_deref(), Some("https://topaz.example"));
    let none = settings(SOURCE, &[]);
    let missing = (policies.credentials)(ProviderId::Topaz, None, None, &none);
    assert!(missing.api_key.is_none());
    let unsupported = (policies.credentials)(ProviderId::Azure, None, None, &topaz_environment);
    assert!(unsupported.api_key.is_none());
    assert!(unsupported.api_base.is_none());
}
