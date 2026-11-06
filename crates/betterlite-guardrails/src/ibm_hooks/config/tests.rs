#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;

use betterlite_config::loader::ConfigLoader;
use rstest::rstest;

use super::{EventHook, record_applied_guardrail, should_run_guardrail};
use crate::ibm_hooks::IbmGuardrail;
use crate::ibm_hooks::test_support::{Mode, config, data, guardrail};
use crate::primitives::json_value::JsonValue;

const DETECTOR_SERVER_URL: &str = "https://detector.test/api/v1/text/contents";

fn guardrail_with(event_hooks: Option<Vec<EventHook>>) -> IbmGuardrail {
    guardrail(
        config(Mode::DetectorServer, DETECTOR_SERVER_URL, true),
        event_hooks,
    )
}

#[rstest]
#[case::unset_runs_every_hook(None, EventHook::PreCall, true)]
#[case::single_hook_matches_its_event(Some(vec![EventHook::PreCall]), EventHook::PreCall, true)]
#[case::single_hook_skips_other_events(Some(vec![EventHook::PreCall]), EventHook::PostCall, false)]
#[case::list_matches_a_member(
    Some(vec![EventHook::DuringCall, EventHook::PostCall]),
    EventHook::DuringCall,
    true
)]
#[case::list_skips_other_events(
    Some(vec![EventHook::DuringCall, EventHook::PostCall]),
    EventHook::PreCall,
    false
)]
#[case::empty_list_matches_nothing(Some(Vec::new()), EventHook::PreCall, false)]
#[case::logging_only_never_matches_a_hook(
    Some(vec![EventHook::LoggingOnly]),
    EventHook::PreCall,
    false
)]
fn should_run_guardrail_follows_the_configured_event_hook(
    #[case] event_hooks: Option<Vec<EventHook>>,
    #[case] event_type: EventHook,
    #[case] expected: bool,
) {
    assert_eq!(
        should_run_guardrail(&guardrail_with(event_hooks), event_type),
        expected
    );
}

#[test]
fn record_applied_guardrail_creates_the_metadata_bucket() {
    let mut prompt = data(r#"{"messages":[]}"#);
    record_applied_guardrail(&mut prompt, "ibm");
    let metadata = prompt.metadata.expect("metadata");
    assert_eq!(metadata.applied_guardrails, Some(vec!["ibm".to_owned()]));
}

#[test]
fn record_applied_guardrail_keeps_existing_names_without_duplicates() {
    let mut prompt = data(r#"{"metadata":{"applied_guardrails":["other","ibm"]}}"#);
    record_applied_guardrail(&mut prompt, "ibm");
    let metadata = prompt.metadata.expect("metadata");
    assert_eq!(
        metadata.applied_guardrails,
        Some(vec!["other".to_owned(), "ibm".to_owned()])
    );
}

#[test]
fn record_applied_guardrail_prefers_litellm_metadata() {
    let mut prompt = data(r#"{"metadata":{"applied_guardrails":["old"]},"litellm_metadata":{}}"#);
    record_applied_guardrail(&mut prompt, "ibm");
    let litellm_metadata = prompt.litellm_metadata.expect("litellm metadata");
    assert_eq!(
        litellm_metadata.applied_guardrails,
        Some(vec!["ibm".to_owned()])
    );
    let metadata = prompt.metadata.expect("metadata");
    assert_eq!(metadata.applied_guardrails, Some(vec!["old".to_owned()]));
}

#[test]
fn record_applied_guardrail_keeps_the_other_metadata_keys() {
    let mut prompt = data(r#"{"metadata":{"user":"ada"}}"#);
    record_applied_guardrail(&mut prompt, "ibm");
    let metadata = prompt.metadata.expect("metadata");
    assert_eq!(
        metadata.other.get("user"),
        Some(&JsonValue::String("ada".to_owned()))
    );
}

#[test]
fn example_config_loads_both_modes() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/ibm-guardrails/example_config.yaml"
    );
    let source = fs::read_to_string(path).expect("example config");
    let lookup = BTreeMap::from([(
        "IBM_GUARDRAILS_AUTH_TOKEN".to_owned(),
        "example-token".to_owned(),
    )]);
    let loaded = ConfigLoader { lookup }
        .load(&source)
        .expect("example config");

    let guardrails = loaded.sections.ibm_guardrails.guardrails;
    assert_eq!(guardrails.len(), 2);
    assert!(guardrails[0].is_detector_server);
    assert!(!guardrails[1].is_detector_server);
    assert_eq!(
        guardrails[0].detector_id,
        Some("jailbreak-detector".to_owned())
    );
    assert_eq!(guardrails[0].score_threshold, Some(0.8_f64));
    assert!(guardrails[0].block_on_detection);
    assert_eq!(guardrails[1].score_threshold, Some(0.7_f64));
    assert!(!guardrails[1].block_on_detection);
}
