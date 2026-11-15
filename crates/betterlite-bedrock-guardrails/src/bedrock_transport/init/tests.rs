#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::content_filter_records::ConfidenceThreshold;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_config::sections::guardrail_translation::bedrock_streaming::SamplingRate;
use rstest::rstest;

use crate::bedrock_transport::test_support::{capture, guardrail_init};
use crate::bedrock_transport::{BedrockGuardrail, BedrockGuardrailError};
use crate::guardrail_checks::ChecksInput;

fn sample_checks() -> ChecksInput {
    ChecksInput::Mapping(BTreeMap::from([(
        "contentFilter".to_owned(),
        GuardrailValue::Object(BTreeMap::new()),
    )]))
}

#[test]
fn init_sets_the_identity_and_streaming_defaults() {
    let guardrail = BedrockGuardrail::__init__(guardrail_init()).expect("the guardrail");
    assert_eq!(guardrail.guardrail_provider, "bedrock");
    assert_eq!(guardrail.guardrail_identifier.as_deref(), Some("gr-1"));
    assert_eq!(guardrail.guardrail_version.as_deref(), Some("1"));
    assert_eq!(guardrail.chunk_budget_chars, 25_000);
    assert_eq!(
        guardrail.content_filter_threshold,
        Some(ConfidenceThreshold(0.5))
    );
    assert!(guardrail.use_native_during_call_hook);
    assert!(!guardrail.disable_exception_on_block);
    assert!(!guardrail.contextual_grounding);
    assert!(guardrail.buffer_until_moderated);
    assert_eq!(guardrail.streaming_sampling_rate, SamplingRate(5));
    assert!(!guardrail.streaming_end_of_stream_only);
    assert!(!guardrail.buffer_release_on_scan);
    assert!(guardrail.checks.is_none());
}

#[test]
fn init_returns_the_five_native_event_hooks() {
    assert_eq!(
        BedrockGuardrail::get_supported_event_hooks(),
        vec![
            GuardrailEventHooks::PreCall,
            GuardrailEventHooks::PostCall,
            GuardrailEventHooks::DuringCall,
            GuardrailEventHooks::PreMcpCall,
            GuardrailEventHooks::DuringMcpCall,
        ]
    );
}

#[test]
fn init_sets_streaming_params_from_the_explicit_options() {
    let mut init = guardrail_init();
    init.buffer_until_moderated = Some(false);
    init.streaming_sampling_rate = Some(SamplingRate(7));
    init.streaming_end_of_stream_only = Some(true);
    init.buffer_release_on_scan = Some(true);
    let guardrail = BedrockGuardrail::__init__(init).expect("the guardrail");
    assert!(!guardrail.buffer_until_moderated);
    assert_eq!(guardrail.streaming_sampling_rate, SamplingRate(7));
    assert!(guardrail.streaming_end_of_stream_only);
    assert!(guardrail.buffer_release_on_scan);
}

#[test]
fn init_rejects_the_two_api_paths_together() {
    let mut init = guardrail_init();
    init.checks = Some(sample_checks());
    let error = BedrockGuardrail::__init__(init).expect_err("both API paths");
    assert!(matches!(error, BedrockGuardrailError::BothApiPaths));
    assert_eq!(
        error.to_string(),
        "Bedrock guardrail accepts either 'guardrailIdentifier'/'guardrailVersion' (ApplyGuardrail) or 'checks' (InvokeGuardrailChecks), not both."
    );
}

#[rstest]
#[case::identifier_only(Some("gr-1"), None)]
#[case::version_only(None, Some("1"))]
fn init_rejects_checks_with_either_half_of_the_identity(
    #[case] identifier: Option<&str>,
    #[case] version: Option<&str>,
) {
    let mut init = guardrail_init();
    init.guardrail_identifier = identifier.map(str::to_owned);
    init.guardrail_version = version.map(str::to_owned);
    init.checks = Some(sample_checks());
    let error = BedrockGuardrail::__init__(init).expect_err("both API paths");
    assert!(matches!(error, BedrockGuardrailError::BothApiPaths));
}

#[test]
fn init_accepts_checks_without_an_identifier() {
    let mut init = guardrail_init();
    init.guardrail_identifier = None;
    init.guardrail_version = None;
    init.checks = Some(sample_checks());
    let guardrail = BedrockGuardrail::__init__(init).expect("the guardrail");
    let normalized = guardrail.checks.expect("the normalized checks");
    assert!(normalized.contains_key("contentFilter"));
}

#[test]
fn init_warns_that_masking_does_nothing_in_checks_mode() {
    let (captured, _guard) = capture();
    let mut init = guardrail_init();
    init.guardrail_identifier = None;
    init.guardrail_version = None;
    init.checks = Some(sample_checks());
    init.mask_request_content = true;
    BedrockGuardrail::__init__(init).expect("the guardrail");
    let warnings = captured.warnings();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("mask_request_content"));
}

#[test]
fn init_warns_that_response_masking_does_nothing_in_checks_mode() {
    let (captured, _guard) = capture();
    let mut init = guardrail_init();
    init.guardrail_identifier = None;
    init.guardrail_version = None;
    init.checks = Some(sample_checks());
    init.mask_response_content = true;
    BedrockGuardrail::__init__(init).expect("the guardrail");
    assert_eq!(captured.warnings().len(), 1);
}

#[test]
fn init_does_not_warn_about_masking_without_checks() {
    let (captured, _guard) = capture();
    let mut init = guardrail_init();
    init.mask_request_content = true;
    init.mask_response_content = true;
    BedrockGuardrail::__init__(init).expect("the guardrail");
    assert!(captured.warnings().is_empty());
}

#[test]
fn init_does_not_warn_when_checks_have_no_masking() {
    let (captured, _guard) = capture();
    let mut init = guardrail_init();
    init.guardrail_identifier = None;
    init.guardrail_version = None;
    init.checks = Some(sample_checks());
    BedrockGuardrail::__init__(init).expect("the guardrail");
    assert!(captured.warnings().is_empty());
}

#[test]
fn init_logs_the_initialization_at_debug_level() {
    let (captured, _guard) = capture();
    BedrockGuardrail::__init__(guardrail_init()).expect("the guardrail");
    let debug_messages: Vec<String> = captured
        .entries()
        .into_iter()
        .filter(|entry| entry.level == tracing::Level::DEBUG)
        .map(|entry| entry.message)
        .collect();
    assert_eq!(debug_messages, vec!["Bedrock Guardrail initialized"]);
}

#[test]
fn init_keeps_the_explicit_timeout() {
    let mut init = guardrail_init();
    init.timeout = Some(Duration::from_secs(30));
    let guardrail = BedrockGuardrail::__init__(init).expect("the guardrail");
    assert_eq!(guardrail.timeout, Some(Duration::from_secs(30)));
}
