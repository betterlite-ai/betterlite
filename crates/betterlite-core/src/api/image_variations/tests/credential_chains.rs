#![cfg(test)]

use betterlite_models::identity::provider_id::ProviderId;
use secrecy::{ExposeSecret as _, SecretString};

use super::super::credentials::{
    DEFAULT_OPENAI_API_BASE, DEFAULT_TOPAZ_API_BASE, VariationPolicies,
};
use super::support::{SOURCE, settings};

#[test]
fn openai_key_falls_through_its_sources() {
    let policies = VariationPolicies::default();
    let all = settings(
        "litellm_settings:\n  api_key: yaml-key\n  openai_key: yaml-openai-key\n",
        &[("OPENAI_API_KEY", "env-key")],
    );
    let supplied = SecretString::from("supplied");
    let key = (policies.credentials)(ProviderId::Openai, Some(&supplied), None, &all)
        .api_key
        .expect("supplied key");
    assert_eq!(key.expose_secret(), "supplied");
    let key = (policies.credentials)(ProviderId::Openai, None, None, &all)
        .api_key
        .expect("global key");
    assert_eq!(key.expose_secret(), "yaml-key");
    let openai_key_only = settings(
        "litellm_settings:\n  openai_key: yaml-openai-key\n",
        &[("OPENAI_API_KEY", "env-key")],
    );
    let key = (policies.credentials)(ProviderId::Openai, None, None, &openai_key_only)
        .api_key
        .expect("openai_key");
    assert_eq!(key.expose_secret(), "yaml-openai-key");
    let environment_only = settings(SOURCE, &[("OPENAI_API_KEY", "env-key")]);
    let key = (policies.credentials)(ProviderId::Openai, None, None, &environment_only)
        .api_key
        .expect("environment key");
    assert_eq!(key.expose_secret(), "env-key");
    let empty = SecretString::from("");
    let key = (policies.credentials)(ProviderId::Openai, Some(&empty), None, &environment_only)
        .api_key
        .expect("an empty key falls through");
    assert_eq!(key.expose_secret(), "env-key");
    let none = settings(SOURCE, &[]);
    assert!(
        (policies.credentials)(ProviderId::Openai, None, None, &none)
            .api_key
            .is_none()
    );
}

#[test]
fn openai_base_falls_through_its_sources() {
    let policies = VariationPolicies::default();
    let all = settings(
        "litellm_settings:\n  api_base: https://yaml.example/v1\n",
        &[
            ("OPENAI_BASE_URL", "https://env.example/v1"),
            ("OPENAI_API_BASE", "https://legacy.example/v1"),
        ],
    );
    let supplied = (policies.credentials)(
        ProviderId::Openai,
        None,
        Some("https://call.example/v1"),
        &all,
    )
    .api_base;
    assert_eq!(supplied.as_deref(), Some("https://call.example/v1"));
    assert_eq!(
        (policies.credentials)(ProviderId::Openai, None, None, &all)
            .api_base
            .as_deref(),
        Some("https://yaml.example/v1")
    );
    let environment_only = settings(
        SOURCE,
        &[
            ("OPENAI_BASE_URL", "https://env.example/v1"),
            ("OPENAI_API_BASE", "https://legacy.example/v1"),
        ],
    );
    assert_eq!(
        (policies.credentials)(ProviderId::Openai, None, None, &environment_only)
            .api_base
            .as_deref(),
        Some("https://env.example/v1")
    );
    let legacy_only = settings(SOURCE, &[("OPENAI_API_BASE", "https://legacy.example/v1")]);
    assert_eq!(
        (policies.credentials)(ProviderId::Openai, None, None, &legacy_only)
            .api_base
            .as_deref(),
        Some("https://legacy.example/v1")
    );
    assert_eq!(
        (policies.credentials)(ProviderId::Openai, None, None, &settings(SOURCE, &[]))
            .api_base
            .as_deref(),
        Some(DEFAULT_OPENAI_API_BASE)
    );
    assert_eq!(
        (policies.credentials)(ProviderId::Openai, None, Some(""), &all)
            .api_base
            .as_deref(),
        Some("https://yaml.example/v1")
    );
}

#[test]
fn topaz_credentials_fall_through_their_sources() {
    let policies = VariationPolicies::default();
    let settings = settings(SOURCE, &[("TOPAZ_API_KEY", "topaz-env-key")]);
    let key = (policies.credentials)(ProviderId::Topaz, None, None, &settings)
        .api_key
        .expect("topaz key");
    assert_eq!(key.expose_secret(), "topaz-env-key");
    assert_eq!(
        (policies.credentials)(ProviderId::Topaz, None, None, &settings)
            .api_base
            .as_deref(),
        Some(DEFAULT_TOPAZ_API_BASE)
    );
    let supplied = SecretString::from("supplied-topaz");
    let key = (policies.credentials)(ProviderId::Topaz, Some(&supplied), None, &settings)
        .api_key
        .expect("supplied topaz key");
    assert_eq!(key.expose_secret(), "supplied-topaz");
    let supplied_base = (policies.credentials)(
        ProviderId::Topaz,
        None,
        Some("https://topaz.example"),
        &settings,
    )
    .api_base;
    assert_eq!(supplied_base.as_deref(), Some("https://topaz.example"));
    let with_base = settings(SOURCE, &[("TOPAZ_API_BASE", "https://topaz.example")]);
    assert_eq!(
        (policies.credentials)(ProviderId::Topaz, None, None, &with_base)
            .api_base
            .as_deref(),
        Some("https://topaz.example")
    );
    let none = settings(SOURCE, &[]);
    let missing = (policies.credentials)(ProviderId::Topaz, None, None, &none);
    assert!(missing.api_key.is_none());
    let unsupported = (policies.credentials)(ProviderId::Azure, None, None, &settings);
    assert!(unsupported.api_key.is_none());
    assert!(unsupported.api_base.is_none());
}
