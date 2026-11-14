#![cfg(test)]

use std::sync::{Arc, Mutex};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::local_ttl_cache::Settings;
use betterlite_context::local_ttl_cache::dual::{Diagnostics, DualLocalCache};
use betterlite_prompts::arguments::json::Object;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use tracing::subscriber::set_default;

use crate::purview_dlp_evaluation::error_handling::{POLICY_BLOCK_STATUS, PurviewDlpHttpError};
use crate::purview_dlp_evaluation::responses_input::responses_api_input_to_str;
use crate::purview_dlp_evaluation::test_support::Captured;

const TRANSFORM_ERROR: &str = "Microsoft Purview DLP: Responses API input could not be transformed for DLP scanning in blocking mode";

fn cache() -> DualLocalCache {
    let diagnostics = Diagnostics {
        redactor: Arc::new(Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap()),
        emit: Arc::new(|_event| {}),
    };
    DualLocalCache::new(
        None,
        None,
        &Settings::default(),
        Arc::new(|| 0.0_f64),
        diagnostics,
    )
}

fn data(raw: &str) -> Object {
    serde_json::from_str(raw).unwrap()
}

fn convert(raw: &str, raise: bool) -> Result<Option<String>, PurviewDlpHttpError> {
    responses_api_input_to_str(&data(raw), &cache(), raise)
}

#[test]
fn a_string_input_is_scanned_as_text() {
    assert_eq!(
        convert(r#"{"input":"hi"}"#, true).unwrap(),
        Some("hi".to_owned())
    );
}

#[test]
fn instructions_are_scanned_before_the_input() {
    assert_eq!(
        convert(r#"{"input":"hi","instructions":"follow"}"#, true).unwrap(),
        Some("follow\n\nhi".to_owned())
    );
}

#[test]
fn instructions_alone_are_scanned() {
    assert_eq!(
        convert(r#"{"instructions":"follow"}"#, true).unwrap(),
        Some("follow".to_owned())
    );
}

#[test]
fn a_null_input_with_instructions_is_scanned() {
    assert_eq!(
        convert(r#"{"input":null,"instructions":"follow"}"#, true).unwrap(),
        Some("follow".to_owned())
    );
}

#[test]
fn a_list_input_is_scanned_message_by_message() {
    assert_eq!(
        convert(
            r#"{"input":[{"role":"user","content":"hi"},{"role":"user","content":"there"}]}"#,
            true
        )
        .unwrap(),
        Some("hi\n\nthere".to_owned())
    );
}

#[test]
fn list_content_parts_are_scanned() {
    assert_eq!(
        convert(
            r#"{"input":[{"role":"user","content":[{"type":"input_text","text":"hi"}]}]}"#,
            true
        )
        .unwrap(),
        Some("hi".to_owned())
    );
}

#[test]
fn a_request_without_input_or_instructions_has_nothing_to_scan() {
    assert_eq!(convert(r#"{"model":"gpt-4"}"#, true).unwrap(), None);
}

#[test]
fn an_empty_string_input_has_nothing_to_scan() {
    assert_eq!(convert(r#"{"input":""}"#, true).unwrap(), None);
}

#[test]
fn an_untransformable_input_raises_in_blocking_mode() {
    let error = convert(r#"{"input":5}"#, true).unwrap_err();
    assert_eq!(error.status, POLICY_BLOCK_STATUS);
    assert_eq!(
        error.detail.get("error"),
        Some(&GuardrailValue::Text(TRANSFORM_ERROR.to_owned()))
    );
}

#[test]
fn an_untransformable_input_is_swallowed_in_audit_mode() {
    assert_eq!(convert(r#"{"input":5}"#, false).unwrap(), None);
}

#[test]
fn a_failed_transform_logs_a_warning_in_audit_mode() {
    let captured = Captured(Arc::new(Mutex::new(Vec::new())));
    let writer = captured.clone();
    let _guard = set_default(
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::WARN)
            .with_writer(move || writer.clone())
            .finish(),
    );

    convert(r#"{"input":true}"#, false).unwrap();

    let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(log.contains("WARN"), "{log}");
    assert!(
        log.contains("Purview DLP: failed to transform responses API input"),
        "{log}"
    );
}
