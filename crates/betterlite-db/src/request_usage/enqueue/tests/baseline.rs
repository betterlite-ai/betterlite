use rust_decimal::Decimal;

use crate::baseline::records::{BaselineEntity, DailyBaselineTarget};
use crate::daily_records::projection::DailyPayload;
use crate::redis_spend::codec::MAX_SPEND_BUFFER_BYTES;
use crate::request_usage::baseline_capture::{
    BASELINE_QUEUE_LIMIT, BaselineCaptureError, BaselineCaptureInput, CaptureOutcome,
    CapturedBaselineError, decode_captured_baseline,
};

use super::support::{
    TurnCase, captured, empty_metadata, metadata_object, turn_metadata, turn_payload,
};

fn daily_payload(text: &str) -> DailyPayload {
    serde_json::from_str(text).unwrap()
}

#[tokio::test]
async fn a_captured_observation_replaces_the_actual_turn() {
    let case = TurnCase::new();
    let capture = captured("sk-capture", "sess-1");
    case.enqueue(&turn_payload(&turn_metadata(Some(&capture))))
        .await;
    assert!(case.turns().is_empty());
    let records = case.baseline_records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].observation.request_id, "req-log");
    assert_eq!(records[0].api_key, "sk-capture");
    assert_eq!(records[0].session_id, "sess-1");
    assert_eq!(records[0].router_name, "auto-router");
    assert_eq!(records[0].baseline_model, "anthropic/claude-opus-5");
    assert!(case.flushed().await);
}

#[tokio::test]
async fn a_capture_naming_another_log_keeps_the_actual_turn() {
    for (api_key, session_id) in [("sk-other", "sess-1"), ("sk-capture", "sess-2")] {
        let case = TurnCase::new();
        let capture = captured(api_key, session_id);
        case.enqueue(&turn_payload(&turn_metadata(Some(&capture))))
            .await;
        assert_eq!(case.turn_count(), 1);
        assert!(case.baseline_records().is_empty());
        assert!(!case.flushed().await);
    }
}

#[tokio::test]
async fn an_unreadable_capture_keeps_the_actual_turn() {
    let case = TurnCase::new();
    let capture = captured("sk-capture", "sess-1")
        .replace(r#""baseline_model":"anthropic/claude-opus-5","#, "");
    case.enqueue(&turn_payload(&turn_metadata(Some(&capture))))
        .await;
    assert_eq!(case.turn_count(), 1);
    assert!(case.baseline_records().is_empty());
}

#[tokio::test]
async fn a_non_text_capture_names_no_observation() {
    let case = TurnCase::new();
    let metadata = r#"{"routing_decision":{"router_model_name":"auto-router","savings_baseline_model":"anthropic/claude-opus-5"},"autorouter_baseline_observation":{"scope":"x"}}"#;
    case.enqueue(&turn_payload(metadata)).await;
    assert_eq!(case.turn_count(), 1);
    assert!(case.baseline_records().is_empty());
}

#[tokio::test]
async fn keeps_the_actual_turn_when_the_baseline_queue_is_at_its_limit() {
    let case = TurnCase::new();
    case.fill_baseline(BASELINE_QUEUE_LIMIT).await;
    let capture = captured("sk-capture", "sess-1");
    case.enqueue(&turn_payload(&turn_metadata(Some(&capture))))
        .await;
    assert_eq!(case.turn_count(), 1);
    assert_eq!(case.baseline_records().len(), BASELINE_QUEUE_LIMIT);
}

#[tokio::test]
async fn publishes_a_capture_just_below_the_baseline_limit() {
    let case = TurnCase::new();
    case.fill_baseline(BASELINE_QUEUE_LIMIT - 1).await;
    let capture = captured("sk-capture", "sess-1");
    case.enqueue(&turn_payload(&turn_metadata(Some(&capture))))
        .await;
    assert!(case.turns().is_empty());
    assert_eq!(case.baseline_records().len(), BASELINE_QUEUE_LIMIT);
}

#[test]
fn a_capture_over_the_buffer_budget_is_refused_before_it_is_parsed() {
    let buffer = vec![b' '; MAX_SPEND_BUFFER_BYTES.saturating_add(1)];
    assert!(matches!(
        decode_captured_baseline(&buffer),
        Err(CapturedBaselineError::TooLarge)
    ));
    assert!(matches!(
        decode_captured_baseline(&buffer[..MAX_SPEND_BUFFER_BYTES]),
        Err(CapturedBaselineError::Json(_))
    ));
}

#[tokio::test]
async fn a_capture_without_the_logs_api_key_or_session_is_rejected() {
    for (api_key, session_id) in [(None, Some("sess-1")), (Some("sk-capture"), None)] {
        let case = TurnCase::new();
        let capture = captured("sk-capture", "sess-1");
        let metadata = empty_metadata();
        let daily = daily_payload("{}");
        let input = BaselineCaptureInput {
            serialized: Some(&capture),
            api_key,
            session_id,
            request_id: Some("req-log"),
            spend: Some(Decimal::ONE),
            turn: None,
            metadata: &metadata,
        };
        let error = case.enqueue_baseline(&daily, &input).await.unwrap_err();
        assert!(matches!(error, BaselineCaptureError::MissingLogIdentity));
    }
}

#[tokio::test]
async fn a_capture_without_the_logs_spend_or_request_id_is_rejected() {
    let case = TurnCase::new();
    let capture = captured("sk-capture", "sess-1");
    let metadata = empty_metadata();
    let daily = daily_payload("{}");
    let without_spend = BaselineCaptureInput {
        serialized: Some(&capture),
        api_key: Some("sk-capture"),
        session_id: Some("sess-1"),
        request_id: Some("req-log"),
        spend: None,
        turn: None,
        metadata: &metadata,
    };
    let error = case
        .enqueue_baseline(&daily, &without_spend)
        .await
        .unwrap_err();
    assert!(matches!(error, BaselineCaptureError::MissingSpend));
    let without_id = BaselineCaptureInput {
        serialized: Some(&capture),
        api_key: Some("sk-capture"),
        session_id: Some("sess-1"),
        request_id: None,
        spend: Some(Decimal::ONE),
        turn: None,
        metadata: &metadata,
    };
    let error = case
        .enqueue_baseline(&daily, &without_id)
        .await
        .unwrap_err();
    assert!(matches!(error, BaselineCaptureError::MissingRequestId));
}

#[tokio::test]
async fn a_capture_whose_evidence_field_is_not_a_mapping_is_rejected() {
    for text in [
        r#"{"routing_decision":5}"#,
        r#"{"routing_decision":"auto-router"}"#,
        r#"{"cost_breakdown":"charged"}"#,
    ] {
        let case = TurnCase::new();
        let capture = captured("sk-capture", "sess-1");
        let metadata = metadata_object(text);
        let daily = daily_payload("{}");
        let input = BaselineCaptureInput {
            serialized: Some(&capture),
            api_key: Some("sk-capture"),
            session_id: Some("sess-1"),
            request_id: Some("req-log"),
            spend: Some(Decimal::ONE),
            turn: None,
            metadata: &metadata,
        };
        let error = case.enqueue_baseline(&daily, &input).await.unwrap_err();
        assert!(matches!(error, BaselineCaptureError::Mapping { .. }));
        assert!(case.baseline_records().is_empty());
    }
}

#[tokio::test]
async fn a_capture_with_falsey_evidence_fields_is_published() {
    let case = TurnCase::new();
    let capture = captured("sk-capture", "sess-1");
    let metadata = metadata_object(r#"{"routing_decision":{},"cost_breakdown":0}"#);
    let daily = daily_payload("{}");
    let input = BaselineCaptureInput {
        serialized: Some(&capture),
        api_key: Some("sk-capture"),
        session_id: Some("sess-1"),
        request_id: Some("req-log"),
        spend: Some(Decimal::ONE),
        turn: None,
        metadata: &metadata,
    };
    let outcome = case.enqueue_baseline(&daily, &input).await.unwrap();
    assert_eq!(outcome, CaptureOutcome::Captured);
    assert_eq!(case.baseline_records().len(), 1);
    assert!(case.flushed().await);
}

#[tokio::test]
async fn a_capture_prices_the_routing_decisions_classifier_cost() {
    let case = TurnCase::new();
    let capture = captured("sk-capture", "sess-1");
    let metadata = metadata_object(r#"{"routing_decision":{"classifier_cost":0.25}}"#);
    let daily = daily_payload("{}");
    let input = BaselineCaptureInput {
        serialized: Some(&capture),
        api_key: Some("sk-capture"),
        session_id: Some("sess-1"),
        request_id: Some("req-log"),
        spend: Some(Decimal::ONE),
        turn: None,
        metadata: &metadata,
    };
    let outcome = case.enqueue_baseline(&daily, &input).await.unwrap();
    assert_eq!(outcome, CaptureOutcome::Captured);
    let records = case.baseline_records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].pricing.classifier_cost, Decimal::new(25, 2));
}

#[tokio::test]
async fn a_published_capture_carries_the_logs_daily_attribution() {
    let case = TurnCase::new();
    let capture = captured("sk-capture", "sess-1");
    let metadata = empty_metadata();
    let daily = daily_payload(
        r#"{"team_id":"t1","startTime":"2025-01-02T03:04:05Z","api_key":"sk-capture","model":"anthropic/claude-opus-5","custom_llm_provider":"anthropic","model_group":"anthropic/claude-opus-5","spend":0.25}"#,
    );
    let input = BaselineCaptureInput {
        serialized: Some(&capture),
        api_key: Some("sk-capture"),
        session_id: Some("sess-1"),
        request_id: Some("req-log"),
        spend: Some(Decimal::ONE),
        turn: None,
        metadata: &metadata,
    };
    let outcome = case.enqueue_baseline(&daily, &input).await.unwrap();
    assert_eq!(outcome, CaptureOutcome::Captured);
    let records = case.baseline_records();
    assert_eq!(records.len(), 1);
    let attributed = records[0]
        .daily
        .as_ref()
        .expect("the daily base names the row");
    assert_eq!(attributed.date, "2025-01-02");
    assert_eq!(attributed.api_key, "sk-capture");
    assert_eq!(attributed.model.as_deref(), Some("anthropic/claude-opus-5"));
    assert_eq!(
        attributed.targets,
        vec![DailyBaselineTarget {
            entity: BaselineEntity::Team,
            entity_id: Some("t1".to_owned()),
        }]
    );
}

#[tokio::test]
async fn a_capture_naming_another_log_publishes_nothing() {
    let case = TurnCase::new();
    let capture = captured("sk-other", "sess-1");
    let metadata = empty_metadata();
    let daily = daily_payload("{}");
    let input = BaselineCaptureInput {
        serialized: Some(&capture),
        api_key: Some("sk-capture"),
        session_id: Some("sess-1"),
        request_id: Some("req-log"),
        spend: Some(Decimal::ONE),
        turn: None,
        metadata: &metadata,
    };
    let outcome = case.enqueue_baseline(&daily, &input).await.unwrap();
    assert_eq!(outcome, CaptureOutcome::Retained);
    assert!(case.baseline_records().is_empty());
}

#[tokio::test]
async fn no_serialized_capture_publishes_nothing() {
    let case = TurnCase::new();
    let metadata = empty_metadata();
    let daily = daily_payload("{}");
    let input = BaselineCaptureInput {
        serialized: None,
        api_key: Some("sk-capture"),
        session_id: Some("sess-1"),
        request_id: Some("req-log"),
        spend: Some(Decimal::ONE),
        turn: None,
        metadata: &metadata,
    };
    let outcome = case.enqueue_baseline(&daily, &input).await.unwrap();
    assert_eq!(outcome, CaptureOutcome::Retained);
    assert!(case.baseline_records().is_empty());
}
