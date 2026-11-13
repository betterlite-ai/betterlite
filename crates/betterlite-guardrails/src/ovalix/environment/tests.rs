#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};

use super::{
    APPLICATION_ID_ENV, POST_CHECKPOINT_ID_ENV, PRE_CHECKPOINT_ID_ENV, TRACKER_API_BASE_ENV,
    TRACKER_API_KEY_ENV, resolve,
};
use crate::ovalix::{CallHook, Params};

fn params() -> Params {
    Params {
        guardrail_name: Some("ovalix".to_owned()),
        tracker_api_base: None,
        tracker_api_key: None,
        application_id: None,
        pre_checkpoint_id: None,
        post_checkpoint_id: None,
        event_hooks: Vec::new(),
        mode: Vec::new(),
        default_on: false,
        timeout: None,
    }
}

fn environment(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn full_environment() -> BTreeMap<String, String> {
    environment(&[
        (TRACKER_API_BASE_ENV, "http://tracker.test"),
        (TRACKER_API_KEY_ENV, "oracle-key"),
        (APPLICATION_ID_ENV, "app-1"),
        (PRE_CHECKPOINT_ID_ENV, "pre-1"),
        (POST_CHECKPOINT_ID_ENV, "post-1"),
    ])
}

#[test]
fn environment_variables_fill_every_unset_setting() {
    let resolved = resolve(&params(), &full_environment(), &[]).expect("resolved");
    assert_eq!(resolved.settings.tracker_api_base, "http://tracker.test");
    assert_eq!(
        resolved.settings.tracker_api_key.expose_secret(),
        "oracle-key"
    );
    assert_eq!(resolved.settings.application_id, "app-1");
    assert_eq!(
        resolved.settings.pre_checkpoint_id.as_deref(),
        Some("pre-1")
    );
    assert_eq!(
        resolved.settings.post_checkpoint_id.as_deref(),
        Some("post-1")
    );
}

#[test]
fn explicit_settings_win_over_the_environment() {
    let mut explicit = params();
    explicit.tracker_api_base = Some("http://explicit.test".to_owned());
    explicit.tracker_api_key = Some(SecretString::from("explicit-key"));
    explicit.application_id = Some("explicit-app".to_owned());
    explicit.pre_checkpoint_id = Some("explicit-pre".to_owned());
    explicit.post_checkpoint_id = Some("explicit-post".to_owned());
    let resolved = resolve(&explicit, &full_environment(), &[]).expect("resolved");
    assert_eq!(resolved.settings.tracker_api_base, "http://explicit.test");
    assert_eq!(
        resolved.settings.tracker_api_key.expose_secret(),
        "explicit-key"
    );
    assert_eq!(resolved.settings.application_id, "explicit-app");
    assert_eq!(
        resolved.settings.pre_checkpoint_id.as_deref(),
        Some("explicit-pre")
    );
}

#[test]
fn blank_explicit_settings_fall_back_to_the_environment() {
    let mut blank = params();
    blank.tracker_api_base = Some(String::new());
    blank.application_id = Some(String::new());
    blank.pre_checkpoint_id = Some(String::new());
    let resolved = resolve(&blank, &full_environment(), &[]).expect("resolved");
    assert_eq!(resolved.settings.tracker_api_base, "http://tracker.test");
    assert_eq!(resolved.settings.application_id, "app-1");
    assert_eq!(
        resolved.settings.pre_checkpoint_id.as_deref(),
        Some("pre-1")
    );
}

#[test]
fn blank_environment_values_count_as_unset() {
    let env = environment(&[(TRACKER_API_BASE_ENV, ""), (TRACKER_API_KEY_ENV, "")]);
    let error = resolve(&params(), &env, &[]).expect_err("missing");
    assert!(
        error
            .missing
            .contains(&"tracker_api_base (OVALIX_TRACKER_API_BASE)")
    );
    assert!(
        error
            .missing
            .contains(&"tracker_api_key (OVALIX_TRACKER_API_KEY)")
    );
}

#[test]
fn every_missing_setting_is_named() {
    let error = resolve(&params(), &BTreeMap::new(), &[]).expect_err("missing");
    assert_eq!(
        error.missing,
        vec![
            "tracker_api_base (OVALIX_TRACKER_API_BASE)",
            "tracker_api_key (OVALIX_TRACKER_API_KEY)",
            "application_id (OVALIX_APPLICATION_ID)",
            "pre_checkpoint_id or post_checkpoint_id",
        ]
    );
}

#[test]
fn base_key_and_application_id_without_a_checkpoint_id_are_missing() {
    let env = environment(&[
        (TRACKER_API_BASE_ENV, "http://tracker.test"),
        (TRACKER_API_KEY_ENV, "oracle-key"),
        (APPLICATION_ID_ENV, "app-1"),
    ]);
    let error = resolve(&params(), &env, &[]).expect_err("missing");
    assert_eq!(
        error.missing,
        vec!["pre_checkpoint_id or post_checkpoint_id"]
    );
}

#[test]
fn a_requested_pre_call_hook_needs_the_pre_checkpoint_id() {
    let env = environment(&[
        (TRACKER_API_BASE_ENV, "http://tracker.test"),
        (TRACKER_API_KEY_ENV, "oracle-key"),
        (APPLICATION_ID_ENV, "app-1"),
        (POST_CHECKPOINT_ID_ENV, "post-1"),
    ]);
    let error = resolve(&params(), &env, &[CallHook::PreCall]).expect_err("missing");
    assert!(
        error
            .missing
            .contains(&"pre_checkpoint_id (OVALIX_PRE_CHECKPOINT_ID)")
    );
}

#[test]
fn a_requested_post_call_hook_needs_the_post_checkpoint_id() {
    let env = environment(&[
        (TRACKER_API_BASE_ENV, "http://tracker.test"),
        (TRACKER_API_KEY_ENV, "oracle-key"),
        (APPLICATION_ID_ENV, "app-1"),
        (PRE_CHECKPOINT_ID_ENV, "pre-1"),
    ]);
    let error = resolve(&params(), &env, &[CallHook::PostCall]).expect_err("missing");
    assert!(
        error
            .missing
            .contains(&"post_checkpoint_id (OVALIX_POST_CHECKPOINT_ID)")
    );
}

#[test]
fn a_pre_checkpoint_id_adds_the_pre_call_hook() {
    let env = environment(&[
        (TRACKER_API_BASE_ENV, "http://tracker.test"),
        (TRACKER_API_KEY_ENV, "oracle-key"),
        (APPLICATION_ID_ENV, "app-1"),
        (PRE_CHECKPOINT_ID_ENV, "pre-1"),
    ]);
    let resolved = resolve(&params(), &env, &[]).expect("resolved");
    assert_eq!(resolved.event_hooks, vec![CallHook::PreCall]);
}

#[test]
fn a_post_checkpoint_id_adds_the_post_call_hook() {
    let env = environment(&[
        (TRACKER_API_BASE_ENV, "http://tracker.test"),
        (TRACKER_API_KEY_ENV, "oracle-key"),
        (APPLICATION_ID_ENV, "app-1"),
        (POST_CHECKPOINT_ID_ENV, "post-1"),
    ]);
    let resolved = resolve(&params(), &env, &[]).expect("resolved");
    assert_eq!(resolved.event_hooks, vec![CallHook::PostCall]);
}

#[test]
fn both_checkpoint_ids_add_both_hooks_in_litellm_order() {
    let resolved = resolve(&params(), &full_environment(), &[]).expect("resolved");
    assert_eq!(
        resolved.event_hooks,
        vec![CallHook::PreCall, CallHook::PostCall]
    );
}

#[test]
fn an_already_requested_hook_is_not_added_twice() {
    let resolved = resolve(&params(), &full_environment(), &[CallHook::PreCall]).expect("resolved");
    assert_eq!(
        resolved.event_hooks,
        vec![CallHook::PreCall, CallHook::PostCall]
    );
}
