use std::collections::BTreeMap;
use std::fs;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use tempfile::NamedTempFile;

use super::support::{
    GUARDRAIL_NAME, MASKED, Mock, Replies, SOCIAL_SECURITY_FOUND, analyzer_requests, block,
    content_of, content_parts, example_recognizers_path, field, guard, guard_args, guard_at,
    logging_only_guard, masking, messages_of, mock, pii_tokens_of, quiet, request, seen, text,
    tokens,
};
use crate::guardrail_base::failure::GuardrailFailure;
use crate::presidio_runtime::PresidioGuard;

type Data = BTreeMap<String, GuardrailValue>;

const CALL_REQUEST: &str =
    r#"{"model":"gpt-4o","messages":[{"role":"user","content":"call me at 555-0100"}]}"#;
const LIST_REQUEST: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":[{"type":"text","text":"call 555-0100"},{"type":"image_url","image_url":{"url":"https://example.test/image.png"}},{"type":"text","text":"or 555-0199"}]},{"role":"assistant","content":"nothing"}]}"#;
const PLAIN_PARTS: &str =
    r#"{"messages":[{"role":"user","content":["plain",{"type":"text","text":"call 555-0100"}]}]}"#;
const OTHER_CONTENT: &str =
    r#"{"messages":[{"role":"user","content":42},{"role":"user","content":null}]}"#;

fn temp_file(contents: &str) -> NamedTempFile {
    let file = NamedTempFile::new().expect("the temp file");
    fs::write(file.path(), contents).expect("the file contents");
    file
}

async fn checked(guard: &PresidioGuard, json: &str) -> Data {
    checked_data(guard, request(json)).await
}

async fn checked_data(guard: &PresidioGuard, mut data: Data) -> Data {
    guard
        .pre_call(&mut data, "completion")
        .await
        .expect("the request is checked");
    data
}

fn output_parse_guard(server: &Mock) -> PresidioGuard {
    let mut args = guard_args(server);
    args.output_parse_pii = Some(true);
    guard(&args)
}

pub(super) async fn masks_string_content() {
    let server = masking();
    let data = checked(&guard_at(&server), CALL_REQUEST).await;
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
    let data = checked(&guard_at(&server), LIST_REQUEST).await;
    let messages = messages_of(&data);
    let parts = content_parts(&messages[0]);
    assert_eq!(text(field(&parts[0], "text")), "<PHONE_NUMBER>");
    assert!(matches!(field(&parts[1], "text"), GuardrailValue::Null));
    assert_eq!(text(field(&parts[1], "type")), "image_url");
    assert_eq!(text(field(&parts[2], "text")), "<PHONE_NUMBER>");
    assert_eq!(
        content_of(&messages[1]),
        GuardrailValue::Text("<PHONE_NUMBER>".to_owned())
    );
    let requests = analyzer_requests(&server);
    let texts: Vec<&str> = requests
        .iter()
        .map(|request| text(field(request, "text")))
        .collect();
    assert_eq!(texts, vec!["call 555-0100", "or 555-0199", "nothing"]);
}

pub(super) async fn skips_other_content() {
    let server = masking();
    let data = checked(&guard_at(&server), OTHER_CONTENT).await;
    assert_eq!(messages_of(&data).len(), 2);
    assert!(seen(&server).is_empty());
}

pub(super) async fn returns_when_messages_are_absent() {
    let server = masking();
    let data = checked(&guard_at(&server), r#"{"model":"gpt-4o"}"#).await;
    assert_eq!(data, request(r#"{"model":"gpt-4o"}"#));
    assert!(seen(&server).is_empty());
}

pub(super) async fn returns_when_messages_are_null() {
    let server = masking();
    let data = checked(&guard_at(&server), r#"{"messages":null}"#).await;
    assert_eq!(data, request(r#"{"messages":null}"#));
    assert!(seen(&server).is_empty());
}

pub(super) async fn skips_non_object_content_parts() {
    let server = masking();
    let data = checked(&guard_at(&server), PLAIN_PARTS).await;
    let parts = content_parts(&messages_of(&data)[0]);
    assert_eq!(text(&parts[0]), "plain");
    assert_eq!(text(field(&parts[1], "text")), "<PHONE_NUMBER>");
    assert_eq!(analyzer_requests(&server).len(), 1);
}

pub(super) async fn fails_when_metadata_is_not_an_object() {
    let server = masking();
    let mut data = request(r#"{"metadata":42,"messages":[]}"#);
    let error = guard_at(&server)
        .pre_call(&mut data, "completion")
        .await
        .expect_err("a number is not a metadata object");
    assert_eq!(error.to_string(), "request metadata must be a JSON object");
    assert!(matches!(error, GuardrailFailure::Http { status: 400, .. }));
}

pub(super) async fn fails_when_messages_are_not_a_list() {
    let server = masking();
    let guard = guard_at(&server);
    let mut data = request(r#"{"messages":"hello"}"#);
    let error = guard
        .pre_call(&mut data, "completion")
        .await
        .expect_err("a string is not a message list");
    assert!(matches!(error, GuardrailFailure::Http { status: 500, .. }));
}

pub(super) async fn skips_a_logging_only_guard() {
    let server = masking();
    let original = request(CALL_REQUEST);
    let data = checked(&logging_only_guard(&guard_args(&server)), CALL_REQUEST).await;
    assert!(seen(&server).is_empty());
    assert_eq!(data, original);
}

pub(super) async fn skips_another_event_hook() {
    let server = masking();
    let mut args = guard_args(&server);
    args.event_hook = LitellmParamsMode::One("post_call".to_owned());
    let data = checked(&guard(&args), CALL_REQUEST).await;
    assert!(seen(&server).is_empty());
    assert_eq!(data, request(CALL_REQUEST));
}

pub(super) async fn skips_a_guardrail_the_request_did_not_ask_for() {
    let server = masking();
    let mut args = guard_args(&server);
    args.default_on = false;
    let data = checked(&guard(&args), CALL_REQUEST).await;
    assert!(seen(&server).is_empty());
    assert_eq!(data, request(CALL_REQUEST));
}

pub(super) async fn runs_a_requested_guardrail_without_default_on() {
    let server = masking();
    let mut args = guard_args(&server);
    args.default_on = false;
    let mut data = request(&format!(
        r#"{{"guardrails":[{{"{GUARDRAIL_NAME}":{{}}}}],"model":"gpt-4o","messages":[{{"role":"user","content":"call me at 555-0100"}}]}}"#
    ));
    guard(&args)
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
    let mut data = request(r#"{"messages":[{"role":"user","content":"1234 is secret"}]}"#);
    let error = guard(&args)
        .pre_call(&mut data, "completion")
        .await
        .expect_err("a blocked entity fails the request");
    assert_eq!(
        error.to_string(),
        format!(
            "Blocked entity detected: US_SSN by Guardrail: {GUARDRAIL_NAME}. This entity is not allowed to be used in this request."
        )
    );
    let GuardrailFailure::BlockedPii {
        entity_type,
        guardrail_name,
        status_code,
    } = &error
    else {
        panic!("expected a blocked entity");
    };
    assert_eq!(entity_type, "US_SSN");
    assert_eq!(guardrail_name.as_deref(), Some(GUARDRAIL_NAME));
    assert_eq!(*status_code, 400);
}

pub(super) async fn per_request_language_reaches_the_analyzer() {
    let server = masking();
    let data = checked(
        &guard_at(&server),
        r#"{"metadata":{"guardrail_config":{"language":"pt"}},"messages":[{"role":"user","content":"call me at 555-0100"}]}"#,
    )
    .await;
    let requests = analyzer_requests(&server);
    let analyze = &requests[0];
    assert_eq!(text(field(analyze, "language")), "pt");
    assert_eq!(
        content_of(&messages_of(&data)[0]),
        GuardrailValue::Text("<PHONE_NUMBER>".to_owned())
    );
}

pub(super) async fn output_parse_pii_stores_tokens() {
    let server = masking();
    let data = checked(&output_parse_guard(&server), CALL_REQUEST).await;
    let messages = messages_of(&data);
    assert_eq!(
        content_of(&messages[0]),
        GuardrailValue::Text("call me at <PHONE_NUMBER_1>".to_owned())
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
    let mut data = request(r#"{"messages":[{"role":"user","content":"zip 90210"}]}"#);
    guard(&args)
        .pre_call(&mut data, "completion")
        .await
        .expect("the request is checked");
    let requests = analyzer_requests(&server);
    let analyze = &requests[0];
    let GuardrailValue::Array(items) = field(analyze, "ad_hoc_recognizers") else {
        panic!("expected the recognizers");
    };
    let names: Vec<&str> = items.iter().map(|item| text(field(item, "name"))).collect();
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
    let mut data = request(r#"{"messages":[{"role":"user","content":"zip 90210"}]}"#);
    guard(&args)
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
    let mut data = request(CALL_REQUEST);
    data.extend(tokens(&[("<OTHER_1>", "kept")]));
    let stored = checked_data(&output_parse_guard(&server), data).await;
    assert_eq!(
        pii_tokens_of(&stored).get("<OTHER_1>").map(String::as_str),
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
    let mut data = request(CALL_REQUEST);
    let error = guard(&args)
        .pre_call(&mut data, "completion")
        .await
        .expect_err("the analyzer failure fails the request");
    assert!(
        error
            .to_string()
            .contains("HTTP 500 from Presidio analyzer: {\"error\":\"boom\"}")
    );
    let GuardrailFailure::GuardrailRaised {
        guardrail_name,
        status_code,
        blocked_content,
        ..
    } = &error
    else {
        panic!("expected a guardrail-raised failure");
    };
    assert_eq!(guardrail_name.as_deref(), Some(GUARDRAIL_NAME));
    assert_eq!(*status_code, 400);
    assert!(!*blocked_content);
}
