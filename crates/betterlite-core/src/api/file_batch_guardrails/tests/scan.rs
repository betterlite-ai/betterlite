#![cfg(test)]

use std::sync::Mutex;

use rstest::rstest;

use super::support::{
    Answer, FakeFailure, FakeHook, Raised, SeenCall, member, named, object, principal, routes,
};
use crate::api::file_batch_guardrails::record_json::dumps::{Rendering, dumps};
use crate::api::file_batch_guardrails::record_json::text::RecordText;
use crate::api::file_batch_guardrails::record_json::{RecordJson, RecordObject};
use crate::api::file_batch_guardrails::records::{
    BatchScanFailure, ParsedRecord, RecordDropped, Redaction, ScanOutcome, UnparseableRecord,
    UnroutableRecord, UnscannableRecord,
};
use crate::api::file_batch_guardrails::scan_record::{ScanContext, scan_record};
use crate::routes::CallType;

const TRUSTED: &str = r#"{"guardrails":["g"],"tags":["t"],"headers":{"x-h":"v"}}"#;

const CHAT: &str = r#"{"custom_id":"a","method":"POST","url":"/v1/chat/completions","body":{"model":"m","messages":[{"role":"user","content":"secret"}],"guardrails":[],"metadata":{"row":1},"litellm_metadata":{"row":2}}}"#;

struct Scanned {
    outcome: Result<Option<ScanOutcome>, FakeFailure>,
    seen: Vec<SeenCall>,
}

async fn scanned(line: &str, answer: Answer) -> Scanned {
    let record = ParsedRecord {
        line_number: 2,
        payload: object(line),
    };
    let hook = FakeHook {
        answer,
        seen: Mutex::new(Vec::new()),
    };
    let scan_metadata = object(TRUSTED);
    let user_api_key_dict = principal();
    let catalog = routes();
    let context = ScanContext {
        scan_metadata: &scan_metadata,
        user_api_key_dict: &user_api_key_dict,
        proxy_logging_obj: &hook,
        routes: &catalog,
    };
    let outcome = scan_record(&record, &context).await;
    Scanned {
        outcome,
        seen: hook.seen.into_inner().unwrap(),
    }
}

fn written(value: &RecordJson) -> String {
    dumps(value, Rendering::AsRead)
}

fn keep(data: RecordObject) -> RecordObject {
    data
}

fn set(data: &mut RecordObject, key: &str, value: &str) {
    data.insert(RecordText::from(key), RecordJson::Object(object(value)));
}

fn redact(mut data: RecordObject) -> RecordObject {
    let messages = object(r#"{"messages":[{"role":"user","content":"[REDACTED]"}]}"#);
    data.insert(
        RecordText::from("messages"),
        member(&messages, "messages").clone(),
    );
    data
}

fn reorder(data: RecordObject) -> RecordObject {
    data.into_iter().rev().collect()
}

fn touch_injected(mut data: RecordObject) -> RecordObject {
    set(&mut data, "metadata", r#"{"guardrail_ran":true}"#);
    set(&mut data, "litellm_metadata", "{}");
    data.insert(RecordText::from("guardrails"), RecordJson::List(Vec::new()));
    data
}

fn drop_injected(mut data: RecordObject) -> RecordObject {
    data.retain(|key, _| *key == RecordText::from("model") || *key == RecordText::from("messages"));
    data
}

fn add_null(mut data: RecordObject) -> RecordObject {
    data.insert(RecordText::from("user"), RecordJson::Null);
    data
}

fn drop_model(mut data: RecordObject) -> RecordObject {
    data.shift_remove(&RecordText::from("model"));
    data
}

fn reroute(mut data: RecordObject) -> RecordObject {
    set(
        &mut data,
        "metadata",
        r#"{"sensitive_data_routing_applied":true,"sensitive_data_routing_guardrail":"router"}"#,
    );
    data
}

fn reroute_unnamed(mut data: RecordObject) -> RecordObject {
    set(
        &mut data,
        "metadata",
        r#"{"sensitive_data_routing_applied":true}"#,
    );
    data
}

fn reroute_not_applied(mut data: RecordObject) -> RecordObject {
    set(
        &mut data,
        "metadata",
        r#"{"sensitive_data_routing_applied":false,"sensitive_data_routing_guardrail":"router"}"#,
    );
    data
}

fn reroute_list_metadata(mut data: RecordObject) -> RecordObject {
    data.insert(RecordText::from("metadata"), RecordJson::List(Vec::new()));
    data
}

fn reroute_in_scan_bag(mut data: RecordObject) -> RecordObject {
    set(
        &mut data,
        "litellm_metadata",
        r#"{"sensitive_data_routing_applied":true}"#,
    );
    data
}

#[tokio::test]
async fn the_hook_sees_the_body_with_trusted_metadata_only() {
    let result = scanned(CHAT, Answer::Rewrite(keep)).await;
    assert_eq!(result.outcome, Ok(None));
    let [call] = result.seen.as_slice() else {
        panic!("expected one hook call, got {:?}", result.seen);
    };
    assert_eq!(call.call_type, CallType::Acompletion);
    assert!(call.guardrails_only);
    assert_eq!(
        written(&RecordJson::Object(call.data.clone())),
        r#"{"model":"m","messages":[{"role":"user","content":"secret"}],"litellm_metadata":{"guardrails":["g"],"tags":["t"],"headers":{"x-h":"v"}},"metadata":{"guardrails":["g"],"tags":["t"],"headers":{"x-h":"v"}}}"#
    );
}

#[tokio::test]
async fn a_rewrite_ships_with_the_records_own_injected_keys() {
    let result = scanned(CHAT, Answer::Rewrite(redact)).await;
    let expected = Redaction {
        line_number: 2,
        custom_id: Some("a".to_owned()),
        text: r#"{"custom_id":"a","method":"POST","url":"/v1/chat/completions","body":{"model":"m","messages":[{"role":"user","content":"[REDACTED]"}],"guardrails":[],"metadata":{"row":1},"litellm_metadata":{"row":2}}}"#.to_owned(),
    };
    assert_eq!(result.outcome, Ok(Some(ScanOutcome::Redaction(expected))));
}

#[tokio::test]
async fn a_rewrite_adds_no_injected_keys_the_record_lacked() {
    let line =
        r#"{"custom_id":7,"url":"/v1/embeddings","body":{"input":"x","encoding_format":"float"}}"#;
    let result = scanned(line, Answer::Rewrite(add_null)).await;
    assert_eq!(
        result.seen.first().map(|call| call.call_type),
        Some(CallType::Aembedding)
    );
    let expected = Redaction {
        line_number: 2,
        custom_id: Some("7".to_owned()),
        text: r#"{"custom_id":7,"url":"/v1/embeddings","body":{"input":"x","encoding_format":"float","user":null}}"#.to_owned(),
    };
    assert_eq!(result.outcome, Ok(Some(ScanOutcome::Redaction(expected))));
}

#[rstest]
#[case::reordered(reorder)]
#[case::only_injected_keys_changed(touch_injected)]
#[case::injected_keys_dropped(drop_injected)]
#[case::rerouted_flag_false(reroute_not_applied)]
#[case::metadata_not_an_object(reroute_list_metadata)]
#[case::flag_only_in_the_scan_bag(reroute_in_scan_bag)]
#[tokio::test]
async fn an_unchanged_record_ships_as_written(#[case] rewrite: fn(RecordObject) -> RecordObject) {
    let result = scanned(CHAT, Answer::Rewrite(rewrite)).await;
    assert_eq!(result.outcome, Ok(None));
}

#[rstest]
#[case::null_key_added(add_null)]
#[case::key_dropped(drop_model)]
#[tokio::test]
async fn a_null_or_missing_key_is_a_change(#[case] rewrite: fn(RecordObject) -> RecordObject) {
    let result = scanned(CHAT, Answer::Rewrite(rewrite)).await;
    assert!(
        matches!(result.outcome, Ok(Some(ScanOutcome::Redaction(_)))),
        "{:?}",
        result.outcome
    );
}

#[rstest]
#[case::named(reroute, Some("router"))]
#[case::unnamed(reroute_unnamed, None)]
#[tokio::test]
async fn a_rerouted_record_cannot_ship(
    #[case] rewrite: fn(RecordObject) -> RecordObject,
    #[case] guardrail: Option<&str>,
) {
    let result = scanned(CHAT, Answer::Rewrite(rewrite)).await;
    let failure = BatchScanFailure::Unroutable(UnroutableRecord {
        line_number: 2,
        custom_id: Some("a".to_owned()),
        guardrail: guardrail.map(str::to_owned),
    });
    assert_eq!(result.outcome, Ok(Some(ScanOutcome::Failure(failure))));
}

#[rstest]
#[case::raised_block(Raised::Verdict { blocked: true, caused: true })]
#[case::intervention(Raised::Status { intervention: true, caused: false })]
#[tokio::test]
async fn a_content_block_drops_the_record(#[case] raised: Raised) {
    let result = scanned(CHAT, Answer::Raise(named(raised))).await;
    let dropped = RecordDropped {
        line_number: 2,
        custom_id: Some("a".to_owned()),
        guardrail: Some("g".to_owned()),
    };
    assert_eq!(result.outcome, Ok(Some(ScanOutcome::Dropped(dropped))));
}

#[rstest]
#[case::raised_without_a_verdict(Raised::Verdict { blocked: false, caused: false })]
#[case::caused_intervention(Raised::Status { intervention: true, caused: true })]
#[case::technical(Raised::Status { intervention: false, caused: false })]
#[tokio::test]
async fn a_failure_to_judge_aborts_the_scan(#[case] raised: Raised) {
    let result = scanned(CHAT, Answer::Raise(named(raised))).await;
    assert_eq!(result.outcome, Err(named(raised)));
}

#[rstest]
#[case::null_body(r#"{"custom_id":"a","url":"/v1/chat/completions","body":null}"#)]
#[case::missing_body(r#"{"custom_id":"a","url":"/v1/chat/completions"}"#)]
#[case::list_body(r#"{"custom_id":"a","url":"/v1/chat/completions","body":[]}"#)]
#[case::text_body(r#"{"custom_id":"a","url":"/v1/chat/completions","body":"{}"}"#)]
#[tokio::test]
async fn a_body_that_is_not_an_object_is_refused_unscanned(#[case] line: &str) {
    let result = scanned(line, Answer::Rewrite(keep)).await;
    let failure = BatchScanFailure::Unparseable(UnparseableRecord { line_number: 2 });
    assert_eq!(result.outcome, Ok(Some(ScanOutcome::Failure(failure))));
    assert!(result.seen.is_empty());
}

#[rstest]
#[case::named_url(
    r#"{"custom_id":"a","url":"/v1/images/generations","body":{"n":1}}"#,
    Some("a"),
    Some("/v1/images/generations")
)]
#[case::number_url(r#"{"custom_id":3,"url":3,"body":{}}"#, Some("3"), None)]
#[case::no_url(r#"{"body":{"model":"m"}}"#, None, None)]
#[case::surrogate_url(r#"{"url":"/x\ud800","body":{}}"#, None, Some("/x?"))]
#[tokio::test]
async fn a_body_without_a_scannable_shape_is_refused_unscanned(
    #[case] line: &str,
    #[case] custom_id: Option<&str>,
    #[case] url: Option<&str>,
) {
    let result = scanned(line, Answer::Rewrite(keep)).await;
    let failure = BatchScanFailure::Unscannable(UnscannableRecord {
        line_number: 2,
        custom_id: custom_id.map(str::to_owned),
        url: url.map(str::to_owned),
    });
    assert_eq!(result.outcome, Ok(Some(ScanOutcome::Failure(failure))));
    assert!(result.seen.is_empty());
}

#[rstest]
#[case::responses(
    r#"{"url":"https://h/v1/responses","body":{"input":"x"}}"#,
    CallType::Aresponses
)]
#[case::messages(
    r#"{"url":"/v1/messages","body":{"messages":[]}}"#,
    CallType::AnthropicMessages
)]
#[case::prompt(r#"{"body":{"prompt":"x"}}"#, CallType::AtextCompletion)]
#[tokio::test]
async fn the_hook_is_called_with_the_resolved_call_type(
    #[case] line: &str,
    #[case] expected: CallType,
) {
    let result = scanned(line, Answer::Rewrite(keep)).await;
    assert_eq!(result.outcome, Ok(None));
    assert_eq!(
        result.seen.first().map(|call| call.call_type),
        Some(expected)
    );
}
