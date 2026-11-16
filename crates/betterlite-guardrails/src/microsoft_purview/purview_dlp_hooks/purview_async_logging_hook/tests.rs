#![cfg(test)]

use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;

use crate::guardrail_records::record::{GuardrailSpanSink, GuardrailStatus};
use crate::microsoft_purview::purview_dlp_hooks::Data;
use crate::microsoft_purview::purview_dlp_hooks::purview_async_logging_hook::{
    AuditEngine, async_logging_hook,
};
use crate::microsoft_purview::purview_dlp_hooks::test_support::{
    Reply, Scan, Script, TestSink, blocking_actions, cache, data, guardrail_at, passing_actions,
    record_base, start, value,
};

const CHAT_RESULT: &str = r#"{"id":"c1","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"hello"}}]}"#;
const AUDITED_KWARGS: &str = r#"{"model":"gpt","messages":[{"role":"user","content":"hello"}],"litellm_params":{"metadata":{"user_api_key_user_id":"user-1"}}}"#;

fn engine(base: &str, sink: &Arc<TestSink>) -> AuditEngine {
    let sink: Arc<dyn GuardrailSpanSink + Send + Sync> = Arc::clone(sink);
    AuditEngine {
        guardrail: Arc::new(guardrail_at(base)),
        record_base: record_base(),
        cache: Arc::new(cache()),
        sink,
    }
}

async fn audit(
    engine: &AuditEngine,
    kwargs: &mut Data,
    result: &GuardrailValue,
    call_type: Option<CallType>,
) {
    async_logging_hook(engine, kwargs, result, call_type).await;
}

#[tokio::test]
async fn audits_the_prompt_and_then_the_response() {
    let mock = start(Script::new(vec![passing_actions(), passing_actions()]));
    let sink = Arc::new(TestSink::default());
    let engine = engine(&mock.base, &sink);
    let mut kwargs = data(AUDITED_KWARGS);
    audit(
        &engine,
        &mut kwargs,
        &value(CHAT_RESULT),
        Some(CallType::Completion),
    )
    .await;
    assert_eq!(
        mock.scanned(),
        vec![
            Scan {
                activity: "uploadText".to_owned(),
                data: "hello".to_owned(),
            },
            Scan {
                activity: "downloadText".to_owned(),
                data: "hello".to_owned(),
            },
        ]
    );
    assert_eq!(sink.records().len(), 2);
}

#[tokio::test]
async fn skips_every_scan_without_a_trusted_identity() {
    let mock = start(Script::passing());
    let sink = Arc::new(TestSink::default());
    let engine = engine(&mock.base, &sink);
    let mut kwargs = data(r#"{"model":"gpt","messages":[{"role":"user","content":"hello"}]}"#);
    audit(
        &engine,
        &mut kwargs,
        &value(CHAT_RESULT),
        Some(CallType::Completion),
    )
    .await;
    assert_eq!(mock.process_calls(), 0);
    assert!(sink.records().is_empty());
}

#[tokio::test]
async fn keeps_auditing_the_response_when_the_prompt_scan_fails() {
    let mock = start(Script::new(vec![
        Reply::status(500, "{}"),
        passing_actions(),
    ]));
    let sink = Arc::new(TestSink::default());
    let engine = engine(&mock.base, &sink);
    let mut kwargs = data(AUDITED_KWARGS);
    audit(
        &engine,
        &mut kwargs,
        &value(CHAT_RESULT),
        Some(CallType::Completion),
    )
    .await;
    assert_eq!(mock.process_calls(), 2);
    assert_eq!(mock.scanned()[1].data, "hello");
}

#[tokio::test]
async fn does_not_raise_when_the_policy_blocks_in_audit_mode() {
    let mock = start(Script::new(vec![blocking_actions(), blocking_actions()]));
    let sink = Arc::new(TestSink::default());
    let engine = engine(&mock.base, &sink);
    let mut kwargs = data(AUDITED_KWARGS);
    audit(
        &engine,
        &mut kwargs,
        &value(CHAT_RESULT),
        Some(CallType::Completion),
    )
    .await;
    let statuses: Vec<GuardrailStatus> = sink
        .records()
        .iter()
        .map(|record| record.guardrail_status)
        .collect();
    assert_eq!(
        statuses,
        vec![
            GuardrailStatus::GuardrailIntervened,
            GuardrailStatus::GuardrailIntervened,
        ]
    );
}

#[tokio::test]
async fn routes_responses_prompts_through_the_input_extractor() {
    let mock = start(Script::new(vec![passing_actions(), passing_actions()]));
    let sink = Arc::new(TestSink::default());
    let engine = engine(&mock.base, &sink);
    let mut kwargs = data(
        r#"{"model":"gpt","instructions":"be careful","input":"hi","litellm_params":{"metadata":{"user_api_key_user_id":"user-1"}}}"#,
    );
    audit(
        &engine,
        &mut kwargs,
        &value(CHAT_RESULT),
        Some(CallType::Aresponses),
    )
    .await;
    assert_eq!(mock.scanned()[0].data, "be careful\n\nhi");
}

#[tokio::test]
async fn skips_the_response_scan_when_the_result_has_no_text() {
    let mock = start(Script::passing());
    let sink = Arc::new(TestSink::default());
    let engine = engine(&mock.base, &sink);
    let mut kwargs = data(AUDITED_KWARGS);
    audit(
        &engine,
        &mut kwargs,
        &value(r#"{"id":"c1","object":"chat.completion","choices":[]}"#),
        Some(CallType::Completion),
    )
    .await;
    assert_eq!(mock.process_calls(), 1);
    assert_eq!(mock.scanned()[0].activity, "uploadText");
}

#[tokio::test]
async fn handles_an_unparseable_responses_result_without_raising() {
    let mock = start(Script::passing());
    let sink = Arc::new(TestSink::default());
    let engine = engine(&mock.base, &sink);
    let mut kwargs = data(AUDITED_KWARGS);
    audit(
        &engine,
        &mut kwargs,
        &value(r#"{"object":"response","id":"r1","created_at":1,"output":"bad"}"#),
        Some(CallType::Completion),
    )
    .await;
    assert_eq!(mock.process_calls(), 1);
    assert_eq!(mock.scanned()[0].activity, "uploadText");
}

#[tokio::test]
async fn keeps_auditing_the_response_when_the_responses_input_cannot_be_transformed() {
    let mock = start(Script::passing());
    let sink = Arc::new(TestSink::default());
    let engine = engine(&mock.base, &sink);
    let mut kwargs = data(
        r#"{"model":"gpt","input":[{"role":"user","content":true}],"litellm_params":{"metadata":{"user_api_key_user_id":"user-1"}}}"#,
    );
    audit(
        &engine,
        &mut kwargs,
        &value(CHAT_RESULT),
        Some(CallType::Aresponses),
    )
    .await;
    assert_eq!(
        mock.scanned(),
        vec![Scan {
            activity: "downloadText".to_owned(),
            data: "hello".to_owned(),
        }]
    );
    assert_eq!(sink.records().len(), 1);
}

#[tokio::test]
async fn audits_a_mixed_text_completion_prompt() {
    let mock = start(Script::new(vec![passing_actions(), passing_actions()]));
    let sink = Arc::new(TestSink::default());
    let engine = engine(&mock.base, &sink);
    let mut kwargs = data(
        r#"{"model":"gpt","prompt":["secret",[1,2]],"litellm_params":{"metadata":{"user_api_key_user_id":"user-1"}}}"#,
    );
    audit(
        &engine,
        &mut kwargs,
        &value(CHAT_RESULT),
        Some(CallType::TextCompletion),
    )
    .await;
    assert_eq!(
        mock.scanned(),
        vec![
            Scan {
                activity: "uploadText".to_owned(),
                data: "secret".to_owned(),
            },
            Scan {
                activity: "downloadText".to_owned(),
                data: "hello".to_owned(),
            },
        ]
    );
}
