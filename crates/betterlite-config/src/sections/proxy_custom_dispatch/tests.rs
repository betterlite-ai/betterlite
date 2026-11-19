#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};

use super::{DEFAULT_ALEPH_ALPHA_BASE, Settings};
use crate::loader::{ConfigError, ConfigLoader};

fn key(secret: &SecretString) -> &str {
    secret.expose_secret()
}

#[test]
fn legacy_provider_loads_keys() {
    let environment = BTreeMap::from([
        (
            "CONFIGURED_ALEPH_KEY".to_owned(),
            "yaml-aleph-key".to_owned(),
        ),
        ("ALEPH_ALPHA_API_KEY".to_owned(), "env-aleph-key".to_owned()),
        ("ALEPHALPHA_API_KEY".to_owned(), "env-tight-key".to_owned()),
        (
            "ALEPH_ALPHA_API_BASE".to_owned(),
            "https://env.example/complete".to_owned(),
        ),
    ]);
    let config = ConfigLoader {
        lookup: environment,
    }
    .load("litellm_settings:\n  aleph_alpha_key: os.environ/CONFIGURED_ALEPH_KEY\n")
    .expect("settings load");
    let settings = config.sections.proxy_custom_dispatch;
    assert_eq!(
        key(settings.aleph_alpha_key.as_ref().expect("yaml key")),
        "yaml-aleph-key"
    );
    assert_eq!(
        key(settings.aleph_alpha_api_key.as_ref().expect("env key")),
        "env-aleph-key"
    );
    assert_eq!(
        key(settings.alephalpha_api_key.as_ref().expect("tight env key")),
        "env-tight-key"
    );
    assert_eq!(
        settings.aleph_alpha_api_base.as_deref(),
        Some("https://env.example/complete")
    );

    let supplied = SecretString::from("supplied-key");
    let global = SecretString::from("global-key");
    assert_eq!(
        key(settings
            .resolve_aleph_alpha_key(Some(&supplied), Some(&global))
            .as_ref()
            .expect("supplied key wins")),
        "supplied-key"
    );
    assert_eq!(
        key(settings
            .resolve_aleph_alpha_key(None, Some(&global))
            .as_ref()
            .expect("configured key beats the global")),
        "yaml-aleph-key"
    );
    assert_eq!(
        settings.resolve_aleph_alpha_base(
            Some("https://supplied.example/complete"),
            Some("https://global.example/complete")
        ),
        "https://supplied.example/complete"
    );
    assert_eq!(
        settings.resolve_aleph_alpha_base(None, Some("https://global.example/complete")),
        "https://global.example/complete"
    );
    assert_eq!(
        settings.resolve_aleph_alpha_base(None, None),
        "https://env.example/complete"
    );
}

#[test]
fn the_environment_spellings_answer_in_order() {
    let environment = BTreeMap::from([
        ("ALEPH_ALPHA_API_KEY".to_owned(), "env-aleph-key".to_owned()),
        ("ALEPHALPHA_API_KEY".to_owned(), "env-tight-key".to_owned()),
    ]);
    let settings = empty_config(environment);
    assert_eq!(
        key(settings
            .resolve_aleph_alpha_key(None, None)
            .as_ref()
            .expect("the first spelling answers")),
        "env-aleph-key"
    );

    let environment =
        BTreeMap::from([("ALEPHALPHA_API_KEY".to_owned(), "env-tight-key".to_owned())]);
    let settings = empty_config(environment);
    assert_eq!(
        key(settings
            .resolve_aleph_alpha_key(None, None)
            .as_ref()
            .expect("the second spelling answers")),
        "env-tight-key"
    );
}

#[test]
fn the_generic_global_key_comes_last() {
    let settings = empty_config(BTreeMap::new());
    let global = SecretString::from("global-key");
    assert_eq!(
        key(settings
            .resolve_aleph_alpha_key(None, Some(&global))
            .as_ref()
            .expect("the global key answers")),
        "global-key"
    );
    assert!(settings.resolve_aleph_alpha_key(None, None).is_none());
}

#[test]
fn an_empty_spelling_falls_through_to_the_next_source() {
    let environment = BTreeMap::from([
        ("ALEPH_ALPHA_API_KEY".to_owned(), String::new()),
        ("ALEPHALPHA_API_KEY".to_owned(), "env-tight-key".to_owned()),
    ]);
    let settings = empty_config(environment);
    let empty = SecretString::from("");
    let global = SecretString::from("global-key");
    assert_eq!(
        key(settings
            .resolve_aleph_alpha_key(Some(&empty), Some(&global))
            .as_ref()
            .expect("the tight spelling answers before the global")),
        "env-tight-key"
    );
    assert_eq!(
        settings.resolve_aleph_alpha_base(Some(""), Some("https://global.example/complete")),
        "https://global.example/complete"
    );
}

#[test]
fn the_base_falls_back_to_the_provider_host() {
    let settings = Settings::default();
    assert_eq!(
        settings.resolve_aleph_alpha_base(None, None),
        DEFAULT_ALEPH_ALPHA_BASE
    );
}

#[test]
fn debug_hides_every_key_spelling() {
    let environment = BTreeMap::from([
        (
            "CONFIGURED_ALEPH_KEY".to_owned(),
            "marker-yaml-key".to_owned(),
        ),
        (
            "ALEPH_ALPHA_API_KEY".to_owned(),
            "marker-env-key".to_owned(),
        ),
        (
            "ALEPHALPHA_API_KEY".to_owned(),
            "marker-tight-key".to_owned(),
        ),
    ]);
    let config = ConfigLoader {
        lookup: environment,
    }
    .load("litellm_settings:\n  aleph_alpha_key: os.environ/CONFIGURED_ALEPH_KEY\n")
    .expect("settings load");
    let settings = config.sections.proxy_custom_dispatch;
    let debug = format!("{settings:?}");
    assert!(!debug.contains("marker-yaml-key"));
    assert!(!debug.contains("marker-env-key"));
    assert!(!debug.contains("marker-tight-key"));
}

#[test]
fn custom_provider_map_stays_unsupported() {
    let error = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("litellm_settings:\n  custom_provider_map: []\n")
    .expect_err("the map has no reader yet");
    assert!(matches!(error, ConfigError::Unsupported { .. }));
}

fn empty_config(environment: BTreeMap<String, String>) -> Settings {
    ConfigLoader {
        lookup: environment,
    }
    .load("litellm_settings: {}\n")
    .expect("settings load")
    .sections
    .proxy_custom_dispatch
}
