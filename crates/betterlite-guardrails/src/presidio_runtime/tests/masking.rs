use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::presidio_runtime::Settings as RuntimeSettings;
use tempfile::NamedTempFile;

use super::support::{
    GUARDRAIL_NAME, MASKED, Replies, SOCIAL_SECURITY_FOUND, analyzer_requests, block, content_of,
    example_recognizers_path, field, guard, guard_args, guard_at, masking, messages_of, mock,
    pii_tokens_of, quiet, request, seen, text, tokens,
};
use crate::guardrail_base::failure::GuardrailFailure;
use crate::presidio_runtime::PresidioDefaults;

const CALL_REQUEST: &str =
    r#"{"model":"gpt-4o","messages":[{"role":"user","content":"call me at 555-0100"}]}"#;

fn temp_file(contents: &str) -> NamedTempFile {
    let file = NamedTempFile::new().expect("the temp file");
    std::fs::write(file.path(), contents).expect("the file contents");
    file
}

pub(super) async fn masks_string_content() {
    let server = masking();
    let guard = guard_at(&server);
    let mut data = request(CALL_REQUEST);
    guard
        .pre_call(&mut data, "completion")
        .await
        .expect("the request is masked");
    let messages = messages_of(&data);
    assert_eq!(
        content_of(&messages[0]),
        GuardrailValue::Text("<PHONE_NUMBER>".to_owned())
    );
    let requests = analyzer_requests(&server);
    let analyze = &requests[0];
    assert_eq!(text(field(analyze, "text")), "call me at 555-0100");
    assert_eq!(text(field(analyze, "language")), "en");
    assert!(matches!(field(analyze, "entities"), GuardrailValue::Null));
    assert!(matches!(
        field(analyze, "ad_hoc_recognizers"),
        GuardrailValue::Null
    ));
}

pub(super) async fn masks_list_content() {
    let server = masking();
    let guard = guard_at(&server);
    let mut data = request(
        r#"{"model":"gpt-4o","messages":[{"role":"user","content":[{"type":"text","text":"call 555-0100"},{"type":"image_url","image_url":{"url":"https://example.test/image.png"}},{"type":"text","text":"or 555-0199"}]},{"role":"assistant","content":"nothing"}]}"#,
    );
    guard
        .pre_call(&mut data, "completion")
        .await
        .expect("the request is masked");
    let messages = messages_of(&data);
    assert_eq!(messages.len(), 2);
    let blocks = content_of(&messages[0]);
    assert_eq!(text(field(&blocks[0], "text")), "<PHONE_NUMBER>");
    assert!(matches!(field(&blocks[1], "text"), GuardrailValue::Null));
    assert_eq!(text(field(&blocks[1], "type")), "image_url");
    assert_eq!(text(field(&blocks[2], "text")), "<PHONE_NUMBER>");
    assert_eq!(
        content_of(&messages[1]),
        GuardrailValue::Text("<PHONE_NUMBER>".to_owned())
    );
    assert_eq!(analyzer_requests(&server).len(), 3);
}

pub(super) async fn skips_other_content() {
    let server = masking();
    let guard = guard_at(&server);
    let mut data =
        request(r#"{"messages":[{"role":"user","content":42},{"role":"user","content":null}]}"#);
    guard
        .pre_call(&mut data, "completion")
        .await
        .expect("nothing is checked");
    assert!(seen(&server).is_empty());
}

pub(super) async fn returns_when_messages_are_absent() {
    let server = masking();
    let guard = guard_at(&server);
    let mut data = request(r#"{"model":"gpt-4o"}"#);
    guard
        .pre_call(&mut data, "completion")
        .await
        .expect("nothing is checked");
    assert!(seen(&server).is_empty());
}

pub(super) async fn fails_when_messages_are_not_a_list() {
    let server = masking();
    let guard = guard_at(&server);
    let mut data = request(r#"{"messages":"hello"}"#);
    let error = guard
        .pre_call(&mut data, "completion")
        .await
        .expect_err("a string is not a message list");
    match error {
        GuardrailFailure::Http { status, .. } => assert_eq!(status, 500),
        other => panic!("expected the message list error, got {other:?}"),
    }
}

pub(super) async fn skips_a_logging_only_guard() {
    let server = masking();
    let args = guard_args(&server);
    let guard = super::support::build(
        &args,
        PresidioDefaults::default(),
        &RuntimeSettings::default(),
        true,
        None,
    )
    .expect("the logging guard");
    let original = request(CALL_REQUEST);
    let mut data = original.clone();
    guard
        .pre_call(&mut data, "completion")
        .await
        .expect("the logging guard skips pre_call");
    assert!(seen(&server).is_empty());
    assert_eq!(data, original);
}

pub(super) async fn skips_another_event_hook() {
    let server = masking();
    let mut args = guard_args(&server);
    args.event_hook = LitellmParamsMode::One("post_call".to_owned());
    let guard = guard(&args);
    let mut data = request(CALL_REQUEST);
    guard
        .pre_call(&mut data, "completion")
        .await
        .expect("the post_call guard skips pre_call");
    assert!(seen(&server).is_empty());
}

pub(super) async fn skips_a_guardrail_the_request_did_not_ask_for() {
    let server = masking();
    let mut args = guard_args(&server);
    args.default_on = false;
    let guard = guard(&args);
    let mut data = request(CALL_REQUEST);
    guard
        .pre_call(&mut data, "completion")
        .await
        .expect("the guardrail was not requested");
    assert!(seen(&server).is_empty());
}

pub(super) async fn runs_a_requested_guardrail_without_default_on() {
    let server = masking();
    let mut args = guard_args(&server);
    args.default_on = false;
    let guard = guard(&args);
    let mut data = request(&format!(
        r#"{{"guardrails":[{{"{GUARDRAIL_NAME}":{{}}}}],"model":"gpt-4o","messages":[{{"role":"user","content":"call me at 555-0100"}}]}}"#
    ));
    guard
        .pre_call(&mut data, "completion")
        .await
        .expect("the requested guardrail masks");
    let messages = messages_of(&data);
    assert_eq!(
        content_of(&messages[0]),
        GuardrailValue::Text("<PHONE_NUMBER>".to_owned())
    );
}

pub(super) async fn blocked_entity_fails_the_request() {
    let server = mock(Replies {
        analyze: (200, SOCIAL_SECURITY_FOUND),
        anonymize: (200, MASKED),
    });
    let mut args = guard_args(&server);
    args.pii_entities_config = Some(block("US_SSN"));
    let guard = guard(&args);
    let mut data = request(r#"{"messages":[{"role":"user","content":"1234 is secret"}]}"#);
    let error = guard
        .pre_call(&mut data, "completion")
        .await
        .expect_err("a blocked entity fails the request");
    assert_eq!(
        error.to_string(),
        format!(
            "Blocked entity detected: US_SSN by Guardrail: {GUARDRAIL_NAME}. This entity is not allowed to be used in this request."
        )
    );
    match error {
        GuardrailFailure::BlockedPii {
            entity_type,
            guardrail_name,
            status_code,
        } => {
            assert_eq!(entity_type, "US_SSN");
            assert_eq!(guardrail_name.as_deref(), Some(GUARDRAIL_NAME));
            assert_eq!(status_code, 400);
        }
        other => panic!("expected a blocked entity, got {other:?}"),
    }
}

pub(super) async fn per_request_language_reaches_the_analyzer() {
    let server = masking();
    let guard = guard_at(&server);
    let mut data = request(
        r#"{"metadata":{"guardrail_config":{"language":"pt"}},"messages":[{"role":"user","content":"call me at 555-0100"}]}"#,
    );
    guard
        .pre_call(&mut data, "completion")
        .await
        .expect("the request is masked");
    let requests = analyzer_requests(&server);
    let analyze = &requests[0];
    assert_eq!(text(field(analyze, "language")), "pt");
}

pub(super) async fn output_parse_pii_stores_tokens() {
    let server = masking();
    let mut args = guard_args(&server);
    args.output_parse_pii = Some(true);
    let guard = guard(&args);
    let mut data = request(CALL_REQUEST);
    guard
        .pre_call(&mut data, "completion")
        .await
        .expect("the request is masked");
    let messages = messages_of(&data);
    assert_eq!(
        content_of(&messages[0]),
        GuardrailValue::Text("<PHONE_NUMBER_1>".to_owned())
    );
    assert_eq!(
        pii_tokens_of(&data),
        BTreeMap::from([("<PHONE_NUMBER_1>".to_owned(), "555-0100".to_owned())])
    );
}

pub(super) async fn example_recognizers_reach_the_analyze_request() {
    let server = quiet();
    let mut args = guard_args(&server);
    args.presidio_ad_hoc_recognizers = Some(example_recognizers_path());
    let guard = guard(&args);
    let mut data = request(r#"{"messages":[{"role":"user","content":"zip 90210"}]}"#);
    guard
        .pre_call(&mut data, "completion")
        .await
        .expect("the request is checked");
    let requests = analyzer_requests(&server);
    let analyze = &requests[0];
    let recognizers = field(analyze, "ad_hoc_recognizers");
    let names: Vec<&str> = match recognizers {
        GuardrailValue::Array(items) => {
            items.iter().map(|item| text(field(item, "name"))).collect()
        }
        other => panic!("expected the recognizers, got {other:?}"),
    };
    assert_eq!(
        names,
        vec!["Zip code Recognizer", "Swiss AHV Number Recognizer"]
    );
}

pub(super) async fn a_null_recognizer_file_adds_no_recognizers() {
    let server = quiet();
    let file = temp_file("null");
    let mut args = guard_args(&server);
    args.presidio_ad_hoc_recognizers = Some(file.path().display().to_string());
    let guard = guard(&args);
    let mut data = request(r#"{"messages":[{"role":"user","content":"zip 90210"}]}"#);
    guard
        .pre_call(&mut data, "completion")
        .await
        .expect("the request is checked");
    let requests = analyzer_requests(&server);
    let analyze = &requests[0];
    assert!(matches!(
        field(analyze, "ad_hoc_recognizers"),
        GuardrailValue::Null
    ));
}

pub(super) async fn keeps_the_tokens_pre_call_stored() {
    let server = masking();
    let mut args = guard_args(&server);
    args.output_parse_pii = Some(true);
    let guard = guard(&args);
    let mut data = request(CALL_REQUEST);
    data.extend(tokens(&[("<OTHER_1>", "kept")]));
    guard
        .pre_call(&mut data, "completion")
        .await
        .expect("the request is masked");
    assert_eq!(
        pii_tokens_of(&data).get("<OTHER_1>").map(String::as_str),
        Some("kept")
    );
}

pub(super) async fn a_guardrail_error_status_is_reported() {
    let server = mock(Replies {
        analyze: (500, r#"{"error":"boom"}"#),
        anonymize: (200, MASKED),
    });
    let mut args = guard_args(&server);
    // PII protection configured makes the analyzer failure fail closed, as LiteLLM does
    args.output_parse_pii = Some(true);
    let guard = guard(&args);
    let mut data = request(CALL_REQUEST);
    let error = guard
        .pre_call(&mut data, "completion")
        .await
        .expect_err("the analyzer failure fails the request");
    assert!(
        error
            .to_string()
            .contains("HTTP 500 from Presidio analyzer: {\"error\":\"boom\"}")
    );
    match error {
        GuardrailFailure::GuardrailRaised {
            guardrail_name,
            status_code,
            blocked_content,
            ..
        } => {
            assert_eq!(guardrail_name.as_deref(), Some(GUARDRAIL_NAME));
            assert_eq!(status_code, 400);
            assert!(!blocked_content);
        }
        other => panic!("expected a guardrail-raised failure, got {other:?}"),
    }
}
