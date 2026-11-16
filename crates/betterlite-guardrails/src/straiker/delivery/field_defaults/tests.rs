#![cfg(test)]

use std::time::Duration;

use crate::generic_guardrail_contracts::UnreachableFallback;
use crate::straiker::delivery::fixtures::{raw, settings};
use crate::straiker::delivery::{
    DEFAULT_INITIAL_BACKOFF, DEFAULT_MAX_BACKOFF, DEFAULT_MAX_PAYLOAD_BYTES, DEFAULT_MAX_RETRIES,
};

#[test]
fn an_unset_timeout_is_five_seconds() {
    assert_eq!(settings(&raw()).timeout, Duration::from_secs(5));
}

#[test]
fn retries_and_backoffs_start_at_their_defaults() {
    let resolved = settings(&raw());
    assert_eq!(resolved.max_retries, DEFAULT_MAX_RETRIES);
    assert_eq!(
        resolved.initial_backoff.to_bits(),
        DEFAULT_INITIAL_BACKOFF.to_bits()
    );
    assert_eq!(
        resolved.max_backoff.to_bits(),
        DEFAULT_MAX_BACKOFF.to_bits()
    );
}

#[test]
fn an_unset_fallback_blocks_and_errors_propagate() {
    let resolved = settings(&raw());
    assert_eq!(
        resolved.unreachable_fallback,
        UnreachableFallback::FailClosed
    );
    assert!(resolved.fail_on_error);
    assert!(!resolved.verbose);
}

#[test]
fn an_unset_payload_limit_is_half_a_mebibyte() {
    assert_eq!(
        settings(&raw()).max_payload_bytes,
        DEFAULT_MAX_PAYLOAD_BYTES
    );
}

#[test]
fn optional_names_start_unset() {
    let resolved = settings(&raw());
    assert_eq!(resolved.guardrail_name, "straiker");
    assert_eq!(resolved.source, "LiteLLM Gateway");
    assert!(resolved.agent_ref.is_none());
    assert!(resolved.client.is_none());
    assert!(resolved.format_hint.is_none());
    assert!(resolved.custom_headers.is_empty());
    assert!(resolved.metadata.is_empty());
}

#[test]
fn the_event_hook_becomes_the_configured_modes() {
    assert_eq!(
        settings(&raw()).configured_modes,
        Some(vec!["pre_call".to_owned()])
    );
}

#[test]
fn an_empty_name_falls_back_to_the_guardrail_name() {
    let mut args = raw();
    args.name = String::new();
    assert_eq!(settings(&args).guardrail_name, "straiker");
}

#[test]
fn an_empty_optional_name_is_unset() {
    let mut args = raw();
    args.agent_ref = Some(String::new());
    args.client = Some(String::new());
    let resolved = settings(&args);
    assert!(resolved.agent_ref.is_none());
    assert!(resolved.client.is_none());
}
