use std::sync::Arc;

use betterlite_db::budget_permission_rows::JsonData;
use rstest::rstest;

use super::support::{Answer, Events, FakeLogger, FakeLoggers, cold_handler, fields, json, seen};
use crate::spend::log_payload::cold_storage::ColdStorageHandler;
use crate::spend::log_payload::content::{RequestResponsePayload, resolve_row_payload};
use crate::spend::visibility::store::PayloadRow;

const KEYED: &str = r#"{"cold_storage_object_key":"2025/01/01/req-1.json"}"#;
const COLD: &str = r#"{"messages":[{"role":"user","content":"cold"}],"response":{"id":"cold-resp"},"proxy_server_request":{"model":"cold-model"}}"#;
const TRUNCATED_AUDIT: &str =
    r#"{"model":"m","classifier_input":{"text":"... (litellm_truncated skipped 40 chars) ..."}}"#;

fn row(
    messages: Option<&str>,
    response: Option<&str>,
    request: Option<&str>,
    metadata: Option<&str>,
) -> PayloadRow {
    PayloadRow {
        request_id: "req-1".to_owned(),
        messages: messages.map(json),
        response: response.map(json),
        proxy_server_request: request.map(json),
        metadata: metadata.map(json),
        user: Some("alice".to_owned()),
        team_id: None,
    }
}

fn payload(
    messages: Option<&str>,
    response: Option<&str>,
    request: Option<&str>,
) -> RequestResponsePayload {
    RequestResponsePayload::from_row(&row(messages, response, request, None))
}

fn cold(answer: Answer) -> (ColdStorageHandler, Events) {
    let events = Events::default();
    let reader = Arc::new(FakeLogger {
        name: "s3_v2",
        answer,
        events: Arc::clone(&events),
    });
    let loggers = FakeLoggers {
        payload: Vec::new(),
        cold: vec![reader],
    };
    (cold_handler(Arc::new(loggers), Some("s3_v2")), events)
}

#[rstest]
#[case::messages(Some(r#"[{"role":"user"}]"#), Some(r#""{}""#), Some(r#""{}""#))]
#[case::response(Some(r#""[]""#), Some(r#"{"id":"r"}"#), None)]
#[case::request(None, Some("null"), Some(r#"{"model":"m"}"#))]
#[case::request_text(Some(r#""""#), None, Some(r#""{\"model\":\"m\"}""#))]
#[tokio::test]
async fn one_stored_field_keeps_cold_storage_closed(
    #[case] messages: Option<&str>,
    #[case] response: Option<&str>,
    #[case] request: Option<&str>,
) {
    let (handler, events) = cold(Answer::Found(COLD.to_owned()));
    let stored = row(messages, response, request, Some(KEYED));
    let resolved = resolve_row_payload(&stored, &handler).await;
    assert_eq!(resolved, payload(messages, response, request));
    assert!(seen(&events).is_empty());
}

#[rstest]
#[case::placeholders(Some(r#""{}""#), Some(r#""{}""#), Some(r#""{}""#))]
#[case::nulls(None, Some("null"), None)]
#[case::empty_containers(Some("[]"), Some("{}"), Some(r#"" [] ""#))]
#[tokio::test]
async fn empty_stored_fields_are_replaced_by_cold_storage(
    #[case] messages: Option<&str>,
    #[case] response: Option<&str>,
    #[case] request: Option<&str>,
) {
    let (handler, events) = cold(Answer::Found(COLD.to_owned()));
    let stored = row(messages, response, request, Some(KEYED));
    let resolved = resolve_row_payload(&stored, &handler).await;
    assert_eq!(
        resolved,
        payload(
            Some(r#"[{"role":"user","content":"cold"}]"#),
            Some(r#"{"id":"cold-resp"}"#),
            Some(r#"{"model":"cold-model"}"#)
        )
    );
    assert_eq!(seen(&events), ["cold s3_v2 2025/01/01/req-1.json"]);
}

#[tokio::test]
async fn a_cold_payload_without_a_field_answers_null_for_it() {
    let (handler, _events) = cold(Answer::Found(r#"{"response":"done"}"#.to_owned()));
    let stored = row(Some(r#""{}""#), None, None, Some(KEYED));
    let resolved = resolve_row_payload(&stored, &handler).await;
    assert_eq!(resolved, payload(None, Some(r#""done""#), None));
}

#[rstest]
#[case::no_key(Some(r#"{"other":1}"#), Answer::Found(COLD.to_owned()), 0)]
#[case::no_metadata(None, Answer::Found(COLD.to_owned()), 0)]
#[case::malformed_metadata(Some(r#""{nope""#), Answer::Found(COLD.to_owned()), 0)]
#[case::missing_object(Some(KEYED), Answer::Missing, 1)]
#[case::failed_read(Some(KEYED), Answer::Fails, 1)]
#[tokio::test]
async fn falls_back_to_the_database_values(
    #[case] metadata: Option<&str>,
    #[case] answer: Answer,
    #[case] reads: usize,
) {
    let (handler, events) = cold(answer);
    let stored = row(Some(r#""{}""#), Some("{}"), None, metadata);
    let resolved = resolve_row_payload(&stored, &handler).await;
    assert_eq!(resolved, payload(Some(r#""{}""#), Some("{}"), None));
    assert_eq!(seen(&events).len(), reads);
}

#[tokio::test]
async fn a_truncated_audit_takes_only_the_cold_audit() {
    let cold_payload = r#"{"messages":"cold","response":"cold","proxy_server_request":{"model":"cold-model","stream":false},"classifier_input":"{\"text\":\"full\"}","originating_request_masked":{"messages":[]}}"#;
    let (handler, events) = cold(Answer::Found(cold_payload.to_owned()));
    let stored = row(
        Some(r#"["db"]"#),
        Some(r#"{"id":"db"}"#),
        Some(TRUNCATED_AUDIT),
        Some(KEYED),
    );
    let resolved = resolve_row_payload(&stored, &handler).await;
    assert_eq!(
        resolved,
        payload(
            Some(r#"["db"]"#),
            Some(r#"{"id":"db"}"#),
            Some(
                r#"{"model":"cold-model","stream":false,"classifier_input":{"text":"full"},"originating_request_masked":{"messages":[]}}"#
            )
        )
    );
    assert_eq!(seen(&events).len(), 1);
}

#[rstest]
#[case::cold_request_not_json(
    r#"{"proxy_server_request":"not json","classifier_input":{"text":"full"}}"#
)]
#[case::cold_request_empty(r#"{"proxy_server_request":{},"classifier_input":{"text":"full"}}"#)]
#[case::cold_request_missing(r#"{"classifier_input":{"text":"full"}}"#)]
#[tokio::test]
async fn a_truncated_audit_overlays_the_stored_snapshot_without_a_cold_request(
    #[case] cold_payload: &str,
) {
    let (handler, _events) = cold(Answer::Found(cold_payload.to_owned()));
    let stored = row(None, None, Some(TRUNCATED_AUDIT), Some(KEYED));
    let resolved = resolve_row_payload(&stored, &handler).await;
    let request = r#"{"model":"m","classifier_input":{"text":"full"}}"#;
    assert_eq!(resolved, payload(None, None, Some(request)));
}

#[tokio::test]
async fn a_truncated_audit_keeps_the_row_when_cold_has_no_audit() {
    let (handler, events) = cold(Answer::Found(COLD.to_owned()));
    let stored = row(Some(r#"["db"]"#), None, Some(TRUNCATED_AUDIT), Some(KEYED));
    let resolved = resolve_row_payload(&stored, &handler).await;
    assert_eq!(
        resolved,
        payload(Some(r#"["db"]"#), None, Some(TRUNCATED_AUDIT))
    );
    assert_eq!(seen(&events).len(), 1);
}

#[rstest]
#[case::marker_without_audit(r#"{"model":"m","note":"litellm_truncated"}"#)]
#[case::audit_without_marker(r#"{"model":"m","classifier_input":{"text":"whole"}}"#)]
#[case::audit_not_an_object(r#"{"model":"m","classifier_input":"litellm_truncated"}"#)]
#[tokio::test]
async fn only_a_truncated_audit_reopens_cold_storage(#[case] request: &str) {
    let (handler, events) = cold(Answer::Found(COLD.to_owned()));
    let stored = row(None, None, Some(request), Some(KEYED));
    let resolved = resolve_row_payload(&stored, &handler).await;
    assert_eq!(resolved, payload(None, None, Some(request)));
    assert!(seen(&events).is_empty());
}

#[tokio::test]
async fn a_cold_audit_overlays_the_cold_request_snapshot() {
    let cold_payload = r#"{"messages":"m","proxy_server_request":"{\"model\":\"cold\",\"classifier_input\":{\"old\":1}}","classifier_input":{"new":2}}"#;
    let (handler, _events) = cold(Answer::Found(cold_payload.to_owned()));
    let stored = row(None, None, None, Some(KEYED));
    let resolved = resolve_row_payload(&stored, &handler).await;
    let request = r#"{"model":"cold","classifier_input":{"new":2}}"#;
    assert_eq!(resolved, payload(Some(r#""m""#), None, Some(request)));
}

#[tokio::test]
async fn a_cold_audit_without_any_snapshot_starts_from_an_empty_request() {
    let cold_payload = r#"{"proxy_server_request":{},"originating_request_masked":"{\"a\":1}"}"#;
    let (handler, _events) = cold(Answer::Found(cold_payload.to_owned()));
    let stored = row(None, None, Some("{}"), Some(KEYED));
    let resolved = resolve_row_payload(&stored, &handler).await;
    let request = r#"{"originating_request_masked":{"a":1}}"#;
    assert_eq!(resolved, payload(None, None, Some(request)));
}

#[tokio::test]
async fn an_injected_cold_reader_wins_over_the_configured_callback() {
    let events = Events::default();
    let configured = Arc::new(FakeLogger {
        name: "s3_v2",
        answer: Answer::Found(COLD.to_owned()),
        events: Arc::clone(&events),
    });
    let injected = Arc::new(FakeLogger {
        name: "injected",
        answer: Answer::Found(r#"{"messages":"injected"}"#.to_owned()),
        events: Arc::clone(&events),
    });
    let loggers = FakeLoggers {
        payload: Vec::new(),
        cold: vec![configured],
    };
    let handler = ColdStorageHandler {
        injected: Some(injected),
        selected: Some("s3_v2".to_owned()),
        loggers: Arc::new(loggers),
    };
    let found = handler.proxy_server_request("k1").await.unwrap();
    assert_eq!(found, Some(fields(r#"{"messages":"injected"}"#)));
    assert_eq!(seen(&events), ["cold injected k1"]);
}

#[rstest]
#[case::nothing_selected(None)]
#[case::selected_but_inactive(Some("gcs_bucket"))]
#[tokio::test]
async fn without_an_active_cold_logger_nothing_is_read(#[case] selected: Option<&str>) {
    let events = Events::default();
    let reader = Arc::new(FakeLogger {
        name: "s3_v2",
        answer: Answer::Found(COLD.to_owned()),
        events: Arc::clone(&events),
    });
    let loggers = FakeLoggers {
        payload: Vec::new(),
        cold: vec![reader],
    };
    let handler = cold_handler(Arc::new(loggers), selected);
    assert_eq!(handler.proxy_server_request("k1").await.unwrap(), None);
    assert!(seen(&events).is_empty());
}

#[tokio::test]
async fn a_failed_cold_read_reaches_the_caller() {
    let (handler, _events) = cold(Answer::Fails);
    handler.proxy_server_request("k1").await.unwrap_err();
}

#[tokio::test]
async fn database_values_are_served_as_stored_json() {
    let (handler, _events) = cold(Answer::Missing);
    let mut stored = row(Some(r#"[{"role":"user"}]"#), None, None, None);
    stored.response = Some(JsonData::String("plain text".to_owned()));
    let resolved = resolve_row_payload(&stored, &handler).await;
    let body = serde_json::to_string(&resolved).unwrap();
    assert_eq!(
        body,
        r#"{"messages":[{"role":"user"}],"response":"plain text","proxy_server_request":null}"#
    );
}
