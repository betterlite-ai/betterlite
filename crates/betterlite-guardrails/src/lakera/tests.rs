#![cfg(test)]

use std::sync::atomic::{AtomicUsize, Ordering};

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::{
    LakeraCategoryThresholds, LakeraEventHook, LegacyLakeraModeration,
    LegacyLakeraModerationParams, ModerationCheck, SUPPORTED_EVENT_HOOKS,
};

fn empty_env(_key: &str) -> String {
    String::new()
}

fn env_with_both(key: &str) -> String {
    match key {
        "LAKERA_API_KEY" => "env-key".to_owned(),
        "LAKERA_API_BASE" => "https://env.example".to_owned(),
        _ => String::new(),
    }
}

#[rstest]
#[case::empty_env_uses_defaults(
    LegacyLakeraModerationParams::default(),
    empty_env,
    ("", "https://api.lakera.ai", ModerationCheck::InParallel),
)]
#[case::env_supplies_key_and_base(
    LegacyLakeraModerationParams::default(),
    env_with_both,
    ("env-key", "https://env.example", ModerationCheck::InParallel),
)]
#[case::explicit_values_win_over_env(
    LegacyLakeraModerationParams {
        moderation_check: Some(ModerationCheck::PreCall),
        category_thresholds: None,
        api_base: Some("https://explicit.example".to_owned()),
        api_key: Some(SecretString::from("explicit-key")),
    },
    env_with_both,
    ("explicit-key", "https://explicit.example", ModerationCheck::PreCall),
)]
fn constructor_resolves_env_defaults_and_overrides(
    #[case] params: LegacyLakeraModerationParams,
    #[case] get_env: fn(&str) -> String,
    #[case] expected: (&str, &str, ModerationCheck),
) {
    let lakera = LegacyLakeraModeration::new(params, get_env, reqwest::Client::default);
    assert_eq!(
        (
            lakera.api_key.expose_secret(),
            lakera.api_base.as_str(),
            lakera.moderation_check,
        ),
        expected
    );
}

#[test]
fn constructor_keeps_configured_category_thresholds() {
    let params = LegacyLakeraModerationParams {
        category_thresholds: Some(LakeraCategoryThresholds {
            prompt_injection: Some(0.5),
            jailbreak: Some(0.9),
        }),
        moderation_check: None,
        api_base: None,
        api_key: None,
    };
    let lakera = LegacyLakeraModeration::new(params, empty_env, reqwest::Client::default);
    assert_eq!(
        lakera.category_thresholds,
        Some(LakeraCategoryThresholds {
            prompt_injection: Some(0.5),
            jailbreak: Some(0.9),
        })
    );
}

#[test]
fn event_hooks_returns_pre_call_and_during_call() {
    assert_eq!(
        SUPPORTED_EVENT_HOOKS,
        [LakeraEventHook::PreCall, LakeraEventHook::DuringCall]
    );
}

static HANDLER_BUILD_COUNT: AtomicUsize = AtomicUsize::new(0);

fn counting_client() -> reqwest::Client {
    HANDLER_BUILD_COUNT.fetch_add(1, Ordering::SeqCst);
    reqwest::Client::default()
}

#[test]
fn handler_is_built_once_in_constructor() {
    let before = HANDLER_BUILD_COUNT.load(Ordering::SeqCst);
    let lakera = LegacyLakeraModeration::new(
        LegacyLakeraModerationParams::default(),
        empty_env,
        counting_client,
    );
    assert_eq!(HANDLER_BUILD_COUNT.load(Ordering::SeqCst), before + 1);
    drop(lakera.async_handler);
}
