use betterlite_core::rules::RuleDecision;
use betterlite_models::decisions::Json;
use rstest::rstest;
use sha2::Digest as _;
use sha2::Sha256;

use std::sync::Arc;

use super::support::Outcome;
use super::support::RecordingBreadcrumbs;
use super::support::chat_request;
use super::support::guardrail_hook;
use super::support::hook;
use super::support::request;
use super::support::setup;
use super::support::setup_message_texts;
use super::support::setup_with;
use super::support::value;
use crate::call_setup::correlation::CorrelationRestore as _;
use crate::call_setup::function_setup::CallSetupError;

fn rejecting_rule() -> Vec<Option<RuleDecision>> {
    vec![Some(RuleDecision::Boolean(false))]
}

#[test]
fn function_setup_leaves_the_chat_logging_state() {
    let mut fixture = setup();
    let result = fixture
        .call(
            "completion",
            &[],
            chat_request(r#"{"id":"fn-1","metadata":{"team":"a"}}"#),
        )
        .expect("the setup succeeds");

    assert_eq!(result.logging.model, "gpt-4o");
    assert_eq!(result.logging.call_type, "completion");
    assert_eq!(result.logging.function_id, "fn-1");
    assert_eq!(result.logging.stream, Some(false));
    assert_eq!(result.logging.user.as_deref(), Some(""));
    assert_eq!(result.logging.applied_guardrails, Some(Vec::new()));
    assert_eq!(
        result.logging.messages,
        Some(vec![value(r#"{"role":"user","content":"hi"}"#)])
    );
    assert_eq!(
        result.logging.litellm_params.get("api_base"),
        Some(&Json::Text(String::new()))
    );
    assert_eq!(
        result.logging.litellm_params.get("metadata"),
        Some(&value(r#"{"team":"a"}"#))
    );
    assert_eq!(
        result.request.get("id"),
        Some(&Json::Text(String::from("fn-1")))
    );
}

#[test]
fn function_setup_keeps_logger_fn_and_pops_callbacks() {
    let mut fixture = setup_with(&[("langfuse", Outcome::Logger(String::from("langfuse")))]);
    let result = fixture
        .call(
            "completion",
            &[],
            chat_request(r#"{"logger_fn":"my-logger","callbacks":["langfuse"]}"#),
        )
        .expect("the setup succeeds");

    assert_eq!(
        result.request.get("logger_fn"),
        Some(&Json::Text(String::from("my-logger")))
    );
    assert!(result.request.get("callbacks").is_none());
}

#[rstest]
#[case::text_completion("text_completion", r#"{"prompt":"hello"}"#, "hello")]
#[case::embedding("embedding", r#"{"input":"hello"}"#, "hello")]
fn function_setup_leaves_a_string_message(
    #[case] call_type: &str,
    #[case] extra: &str,
    #[case] text: &str,
) {
    let mut fixture = setup();
    let result = fixture
        .call(call_type, &[], chat_request(extra))
        .expect("the setup succeeds");

    assert_eq!(
        result.logging.messages,
        Some(vec![value(&format!(
            r#"{{"role":"user","content":"{text}"}}"#
        ))])
    );
    assert_eq!(result.logging.call_type, call_type);
}

#[rstest]
#[case::stream_flag(r#"{"stream":true}"#, "completion")]
#[case::genai_stream("{}", "generate_content_stream")]
fn function_setup_marks_a_stream(#[case] extra: &str, #[case] call_type: &str) {
    let mut fixture = setup();
    let result = fixture
        .call(call_type, &[], chat_request(extra))
        .expect("the setup succeeds");

    assert_eq!(result.logging.stream, Some(true));
}

#[test]
fn function_setup_leaves_an_unstreamed_call() {
    let mut fixture = setup();
    let result = fixture
        .call("completion", &[], chat_request(r#"{"stream":false}"#))
        .expect("the setup succeeds");

    assert_eq!(result.logging.stream, Some(false));
}

#[test]
fn function_setup_failing_pre_call_rule_stops_the_call() {
    let mut fixture = setup();
    let error = fixture
        .call("completion", &rejecting_rule(), chat_request("{}"))
        .expect_err("the pre-call rule rejects the call");

    assert!(matches!(error, CallSetupError::Rule(_)));
    assert!(fixture.lists.success.is_empty());
}

#[test]
fn function_setup_pre_call_rule_reads_only_string_content() {
    let mut fixture = setup();
    let error = fixture
        .call(
            "completion",
            &rejecting_rule(),
            chat_request(
                r#"{"messages":[{"role":"user","content":[{"type":"text","text":"hi"}]}]}"#,
            ),
        )
        .expect_err("an empty buffer still reaches the rule");

    assert!(matches!(error, CallSetupError::Rule(_)));
}

#[test]
fn function_setup_pre_call_rule_is_skipped_without_content() {
    let mut fixture = setup();
    fixture
        .call(
            "completion",
            &rejecting_rule(),
            chat_request(r#"{"messages":[{"role":"user"}]}"#),
        )
        .expect("the rule sees no message content");

    assert!(fixture.lists.success.is_empty());
}

#[test]
fn function_setup_strips_thought_signatures_from_the_request_only() {
    let mut fixture = setup();
    let result = fixture
        .call(
            "completion",
            &[],
            chat_request(
                r#"{"custom_llm_provider":"openai","messages":[{"role":"assistant","tool_calls":[{"id":"c1__thought__sig"}]},{"role":"tool","tool_call_id":"c2__thought__sig"}]}"#,
            ),
        )
        .expect("the setup succeeds");

    assert_eq!(
        result.request.get("messages"),
        Some(&value(
            r#"[{"role":"assistant","tool_calls":[{"id":"c1"}]},{"role":"tool","tool_call_id":"c2"}]"#
        ))
    );
    assert_eq!(
        result.logging.messages,
        Some(vec![
            value(r#"{"role":"assistant","tool_calls":[{"id":"c1__thought__sig"}]}"#),
            value(r#"{"role":"tool","tool_call_id":"c2__thought__sig"}"#),
        ])
    );
}

#[test]
fn function_setup_keeps_thought_signatures_for_a_gemini_target() {
    let mut fixture = setup();
    let result = fixture
        .call(
            "completion",
            &[],
            chat_request(
                r#"{"custom_llm_provider":"gemini","messages":[{"role":"tool","tool_call_id":"c1__thought__sig"}]}"#,
            ),
        )
        .expect("the setup succeeds");

    assert_eq!(
        result.request.get("messages"),
        Some(&value(
            r#"[{"role":"tool","tool_call_id":"c1__thought__sig"}]"#
        ))
    );
}

#[test]
fn function_setup_sync_call_leaves_correlation_alone() {
    let mut fixture = setup();
    fixture.correlation_settings.request_correlation_in_logs = true;
    fixture.is_async_call = false;
    fixture
        .call("completion", &[], chat_request("{}"))
        .expect("the setup succeeds");

    assert_eq!(fixture.correlation.trace_id, "");
    assert_eq!(fixture.correlation.session_id, "");
}

#[test]
fn function_setup_async_call_stamps_correlation() {
    let mut fixture = setup();
    fixture.correlation_settings.request_correlation_in_logs = true;
    fixture
        .call("completion", &[], chat_request("{}"))
        .expect("the setup succeeds");

    assert_eq!(fixture.correlation.trace_id, "trace-1");
}

#[test]
fn function_setup_correlation_restore_returns_the_pre_call_ids() {
    let mut fixture = setup();
    fixture.correlation_settings.request_correlation_in_logs = true;
    let result = fixture
        .call("completion", &[], chat_request("{}"))
        .expect("the setup succeeds");

    result
        .logging
        .restore_correlation_context(&mut fixture.correlation);

    assert_eq!(fixture.correlation.trace_id, "");
}

#[test]
fn function_setup_applied_guardrails_reach_the_logging_state() {
    let mut fixture = setup();
    fixture.callbacks = vec![
        guardrail_hook("g1", Some("default"), true),
        guardrail_hook("g2", Some("requested"), false),
        guardrail_hook("g3", Some("off"), false),
        guardrail_hook("g4", None, true),
        hook("plain"),
    ];
    let result = fixture
        .call(
            "completion",
            &[],
            chat_request(r#"{"metadata":{"requester_metadata":{"guardrails":["requested"]}}}"#),
        )
        .expect("the setup succeeds");

    assert_eq!(
        result.logging.applied_guardrails,
        Some(vec![String::from("default"), String::from("requested")])
    );
}

#[test]
fn function_setup_breadcrumb_records_the_request() {
    let mut fixture = setup();
    let breadcrumbs = Arc::new(RecordingBreadcrumbs::default());
    fixture.breadcrumbs = Some(Arc::clone(&breadcrumbs));
    fixture
        .call("completion", &[], chat_request("{}"))
        .expect("the setup succeeds");

    let entries = breadcrumbs.entries.lock().expect("the lock is free");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, "litellm.llm_call");
    assert_eq!(entries[0].2, "info");
    assert!(entries[0].1.contains("Keyword Args:"));
    assert!(entries[0].1.contains("gpt-4o"));
    drop(entries);
}

#[test]
fn function_setup_breadcrumb_hides_messages_when_turned_off() {
    let mut fixture = setup();
    fixture.turn_off_message_logging = true;
    let breadcrumbs = Arc::new(RecordingBreadcrumbs::default());
    fixture.breadcrumbs = Some(Arc::clone(&breadcrumbs));
    fixture
        .call("completion", &[], chat_request(r#"{"input":"hidden"}"#))
        .expect("the setup succeeds");

    let entries = breadcrumbs.entries.lock().expect("the lock is free");
    assert!(!entries[0].1.contains("messages"));
    assert!(!entries[0].1.contains("hidden"));
    assert!(entries[0].1.contains("gpt-4o"));
    drop(entries);
}

#[test]
fn function_setup_requires_a_model() {
    let mut fixture = setup();
    let error = fixture
        .call("completion", &[], request(r#"{"messages":[]}"#))
        .expect_err("a call without a model is invalid");

    assert_eq!(error.to_string(), "the model value is required");
}

#[test]
fn function_setup_requires_a_call_id() {
    let mut fixture = setup();
    let error = fixture
        .call(
            "completion",
            &[],
            request(r#"{"model":"gpt-4o","messages":[]}"#),
        )
        .expect_err("a call without an id is invalid");

    assert_eq!(error.to_string(), "the litellm_call_id value is required");
}

#[test]
fn function_setup_reports_a_trusted_variable_type() {
    let mut fixture = setup();
    let error = fixture
        .call(
            "completion",
            &[],
            chat_request(r#"{"litellm_trusted_callback_vars":{"dd_api_key":7}}"#),
        )
        .expect_err("a number is not a trusted variable");

    assert_eq!(
        error.to_string(),
        "the litellm_trusted_callback_vars value must be an object of strings"
    );
}

#[test]
fn function_setup_hides_trusted_variable_values_in_debug() {
    let mut fixture = setup();
    let result = fixture
        .call(
            "completion",
            &[],
            chat_request(r#"{"litellm_trusted_callback_vars":{"dd_api_key":"secret-value"}}"#),
        )
        .expect("the setup succeeds");

    assert_eq!(result.logging.trusted_callback_vars.len(), 1);
    let debug = format!("{:?}", result.logging.trusted_callback_vars);
    assert!(!debug.contains("secret-value"));
}

#[test]
fn function_setup_registers_custom_providers() {
    let mut fixture = setup();
    fixture.providers.custom_provider_map = vec![String::from("myllm"), String::from("myllm")];
    fixture.providers.providers = vec![String::from("openai"), String::from("myllm")];
    fixture
        .call("completion", &[], chat_request("{}"))
        .expect("the setup succeeds");

    assert_eq!(
        fixture.providers.providers,
        vec![String::from("openai"), String::from("myllm")]
    );
    assert_eq!(
        fixture.providers.custom_providers,
        vec![String::from("myllm")]
    );
}

#[test]
fn function_setup_litellm_metadata_fills_an_absent_metadata() {
    let mut fixture = setup();
    let result = fixture
        .call(
            "completion",
            &[],
            chat_request(r#"{"litellm_metadata":{"team":"b"}}"#),
        )
        .expect("the setup succeeds");

    let params = &result.logging.litellm_params;
    assert_eq!(
        params.get("litellm_metadata"),
        Some(&value(r#"{"team":"b"}"#))
    );
    assert_eq!(params.get("metadata"), Some(&value(r#"{"team":"b"}"#)));
}

#[test]
fn function_setup_stream_options_reach_the_logging_state() {
    let mut fixture = setup();
    let result = fixture
        .call(
            "completion",
            &[],
            chat_request(r#"{"stream_options":{"include_usage":true}}"#),
        )
        .expect("the setup succeeds");

    assert_eq!(
        result.logging.stream_options,
        Some(value(r#"{"include_usage":true}"#))
    );
}

#[test]
fn function_setup_transcription_hashes_the_file_text() {
    let mut fixture = setup();
    let result = fixture
        .call(
            "transcription",
            &[],
            request(r#"{"model":"whisper-1","file":"audio.wav","litellm_call_id":"call-1"}"#),
        )
        .expect("the setup succeeds");

    let checksum = hex::encode(Sha256::digest(b"audio.wav"));
    assert_eq!(
        result.logging.messages,
        Some(vec![value(&format!(
            r#"{{"role":"user","content":"{checksum}"}}"#
        ))])
    );
    assert_eq!(
        result.request.get("metadata"),
        Some(&value(&format!(r#"{{"file_checksum":"{checksum}"}}"#)))
    );
}

#[test]
fn function_setup_transcription_requires_a_file() {
    let mut fixture = setup();
    let error = fixture
        .call(
            "transcription",
            &[],
            request(r#"{"model":"whisper-1","litellm_call_id":"call-1"}"#),
        )
        .expect_err("a transcription call needs a file");

    assert_eq!(error.to_string(), "the file value is required");
}

#[test]
fn function_setup_ocr_summarizes_the_document() {
    let mut fixture = setup();
    let result = fixture
        .call(
            "ocr",
            &[],
            chat_request(r#"{"document":{"document_url":"https://example.com/a.png"}}"#),
        )
        .expect("the setup succeeds");

    assert_eq!(
        result.logging.messages,
        Some(vec![value(
            r#"{"role":"user","content":"https://example.com/a.png"}"#
        )])
    );
}

#[rstest]
#[case::responses("responses", r#"{"input":"hello"}"#, "hello")]
#[case::speech("speech", "{}", "speech")]
fn function_setup_defaults_the_call_type_message(
    #[case] call_type: &str,
    #[case] extra: &str,
    #[case] expected: &str,
) {
    assert_eq!(setup_message_texts(call_type, extra), [expected]);
}

#[test]
fn function_setup_search_joins_string_parts() {
    let mut fixture = setup();
    let result = fixture
        .call(
            "search",
            &[],
            chat_request(r#"{"query":["first","second"]}"#),
        )
        .expect("the setup succeeds");

    assert_eq!(
        result.logging.messages,
        Some(vec![value(r#"{"role":"user","content":"first\nsecond"}"#)])
    );
}

#[test]
fn function_setup_decision_state_is_serialized() {
    let mut fixture = setup();
    let result = fixture
        .call(
            "decisions",
            &[],
            chat_request(r#"{"state":{"question":"why"}}"#),
        )
        .expect("the setup succeeds");

    assert_eq!(
        result.logging.messages,
        Some(vec![value(
            r#"{"role":"user","content":"{\"question\":\"why\"}"}"#
        )])
    );
}

#[rstest]
#[case::unknown("some_new_call", "default-message-value")]
#[case::vector_store("vector_store_list", "")]
fn function_setup_falls_back_on_the_call_type(#[case] call_type: &str, #[case] content: &str) {
    let mut fixture = setup();
    let result = fixture
        .call(call_type, &[], chat_request("{}"))
        .expect("the setup succeeds");

    if content.is_empty() {
        assert_eq!(result.logging.messages, Some(Vec::new()));
    } else {
        assert_eq!(
            result.logging.messages,
            Some(vec![value(&format!(
                r#"{{"role":"user","content":"{content}"}}"#
            ))])
        );
    }
}

#[test]
fn function_setup_leaves_a_rerank_query() {
    let mut fixture = setup();
    let result = fixture
        .call(
            "rerank",
            &[],
            chat_request(r#"{"query":"what is a reranker"}"#),
        )
        .expect("the setup succeeds");

    assert_eq!(
        result.logging.messages,
        Some(vec![value(
            r#"{"role":"user","content":"what is a reranker"}"#
        )])
    );
}

#[test]
fn function_setup_leaves_an_image_edit_prompt() {
    let mut fixture = setup();
    let result = fixture
        .call("image_edit", &[], chat_request(r#"{"prompt":"add a hat"}"#))
        .expect("the setup succeeds");

    assert_eq!(
        result.logging.messages,
        Some(vec![value(r#"{"role":"user","content":"add a hat"}"#)])
    );
}

#[test]
fn function_setup_leaves_a_moderation_input() {
    let mut fixture = setup();
    let result = fixture
        .call(
            "moderation",
            &[],
            chat_request(r#"{"input":"please review"}"#),
        )
        .expect("the setup succeeds");

    assert_eq!(
        result.logging.messages,
        Some(vec![value(r#"{"role":"user","content":"please review"}"#)])
    );
}

#[test]
fn function_setup_null_stream_options_stays_absent() {
    let mut fixture = setup();
    let result = fixture
        .call(
            "completion",
            &[],
            chat_request(r#"{"stream_options":null}"#),
        )
        .expect("the setup succeeds");

    assert_eq!(result.logging.stream_options, None);
}

#[test]
fn function_setup_collects_callback_params_from_the_request() {
    let mut fixture = setup();
    let result = fixture
        .call(
            "completion",
            &[],
            chat_request(r#"{"langsmith_project":"project"}"#),
        )
        .expect("the setup succeeds");

    assert_eq!(
        result
            .logging
            .callback_dynamic_params
            .get("langsmith_project"),
        Some(&Json::Text(String::from("project")))
    );
}
