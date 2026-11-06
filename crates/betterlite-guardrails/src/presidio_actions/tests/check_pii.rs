#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use axum::http::StatusCode;
use betterlite_config::sections::guardrail_identity::enums::PiiAction;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use serde_json::value::RawValue;

use super::shared::{
    MockServer, NO_PARAMS, analyze_requests, analyzer, anonymize_requests, anonymizer, json_reply,
    json_reply_with, logged_records, request_data, seen_item, spawn_analyzer, spawn_anonymizer,
    tokens,
};
use crate::presidio_actions::check::CheckPiiCall;
use crate::presidio_actions::error::PresidioActionsError;
use crate::presidio_actions::request_data::{PresidioPerRequestConfig, RequestData};

const TWO_DETECTIONS: &str = r#"[{"entity_type":"PERSON","start":0,"end":5,"score":0.9},{"entity_type":"EMAIL_ADDRESS","start":6,"end":23,"score":0.2}]"#;
const MASKED_REPLY: &str = r#"{"text":"<PERSON> <EMAIL_ADDRESS>","items":[{"entity_type":"PERSON"},{"entity_type":"EMAIL_ADDRESS"}]}"#;

fn no_entities() -> BTreeMap<String, PiiAction> {
    BTreeMap::new()
}

struct FilteredRun {
    analyzer_mock: MockServer,
    anonymizer_mock: MockServer,
    data: RequestData,
    text: String,
}

async fn run_filtered_check() -> FilteredRun {
    let analyzer_mock = spawn_analyzer(|_| json_reply(TWO_DETECTIONS), Duration::ZERO).await;
    let anonymizer_mock = spawn_anonymizer(|_| json_reply(MASKED_REPLY), Duration::ZERO).await;
    let mut analyzer = analyzer(&analyzer_mock.base);
    analyzer.settings.score_thresholds = BTreeMap::from([("EMAIL_ADDRESS".to_owned(), 0.5_f64)]);
    let client = anonymizer(&anonymizer_mock.base);
    let mut data = request_data("{}");
    let entities = no_entities();
    let text = client
        .check_pii(
            &analyzer,
            CheckPiiCall {
                text: "Alice alice@example.com",
                output_parse_pii: false,
                presidio_config: None,
                dynamic_params: &NO_PARAMS,
                pii_entities_config: &entities,
                request_data: &mut data,
            },
        )
        .await
        .unwrap();
    FilteredRun {
        analyzer_mock,
        anonymizer_mock,
        data,
        text,
    }
}

#[tokio::test]
async fn analyzes_filters_and_forwards_only_the_kept_detections() {
    let run = run_filtered_check().await;
    assert_eq!(run.text, "<PERSON> <EMAIL_ADDRESS>");
    let seen = analyze_requests(&run.analyzer_mock);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].text, "Alice alice@example.com");
    assert_eq!(seen[0].language.as_deref(), Some("en"));
    let anonymized = anonymize_requests(&run.anonymizer_mock);
    assert_eq!(anonymized.len(), 1);
    assert_eq!(anonymized[0].analyzer_results.len(), 1);
    assert_eq!(
        seen_item(&anonymized[0].analyzer_results[0])
            .entity_type
            .as_deref(),
        Some("PERSON")
    );
}

#[tokio::test]
async fn records_the_entity_counts_and_status() {
    let run = run_filtered_check().await;
    let records = logged_records(&run.data);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].guardrail_status, "success");
    assert_eq!(records[0].guardrail_name, "presidio-global");
    assert_eq!(records[0].guardrail_provider, "presidio");
    assert_eq!(records[0].masked_entity_count.get("PERSON"), Some(&1));
    assert_eq!(
        records[0].masked_entity_count.get("EMAIL_ADDRESS"),
        Some(&1)
    );
    assert!(records[0].end_time >= records[0].start_time);
    assert!(records[0].duration >= 0.0_f64);
    let response: Vec<Box<RawValue>> =
        serde_json::from_str(records[0].guardrail_response.get()).unwrap();
    assert_eq!(response.len(), 1);
}

#[tokio::test]
async fn deny_listed_detections_never_reach_the_anonymizer() {
    let analyzer_mock = spawn_analyzer(|_| json_reply(TWO_DETECTIONS), Duration::ZERO).await;
    let anonymizer_mock = spawn_anonymizer(
        |_| json_reply(r#"{"text":"clean","items":[]}"#),
        Duration::ZERO,
    )
    .await;
    let mut analyzer = analyzer(&analyzer_mock.base);
    analyzer.settings.entities_deny_list = vec!["EMAIL_ADDRESS".to_owned()];
    let client = anonymizer(&anonymizer_mock.base);
    let mut data = request_data("{}");
    let entities = no_entities();
    let text = client
        .check_pii(
            &analyzer,
            CheckPiiCall {
                text: "Alice alice@example.com",
                output_parse_pii: false,
                presidio_config: None,
                dynamic_params: &NO_PARAMS,
                pii_entities_config: &entities,
                request_data: &mut data,
            },
        )
        .await
        .unwrap();
    assert_eq!(text, "clean");
    let anonymized = anonymize_requests(&anonymizer_mock);
    assert_eq!(anonymized.len(), 1);
    assert_eq!(anonymized[0].analyzer_results.len(), 1);
    assert_eq!(
        seen_item(&anonymized[0].analyzer_results[0])
            .entity_type
            .as_deref(),
        Some("PERSON")
    );
}

#[tokio::test]
async fn a_blocked_entity_raises_before_the_anonymizer_runs() {
    let analyzer_mock = spawn_analyzer(|_| json_reply(TWO_DETECTIONS), Duration::ZERO).await;
    let anonymizer_mock = spawn_anonymizer(
        |_| json_reply(r#"{"text":"should not run"}"#),
        Duration::ZERO,
    )
    .await;
    let analyzer = analyzer(&analyzer_mock.base);
    let client = anonymizer(&anonymizer_mock.base);
    let mut data = request_data("{}");
    let entities = BTreeMap::from([("PERSON".to_owned(), PiiAction::Block)]);
    let error = client
        .check_pii(
            &analyzer,
            CheckPiiCall {
                text: "Alice alice@example.com",
                output_parse_pii: false,
                presidio_config: None,
                dynamic_params: &NO_PARAMS,
                pii_entities_config: &entities,
                request_data: &mut data,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        PresidioActionsError::BlockedEntity { entity_type, .. } if entity_type == "PERSON"
    ));
    assert!(anonymizer_mock.requests.lock().unwrap().is_empty());
    let records = logged_records(&data);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].guardrail_status, "guardrail_failed_to_respond");
    let message: String = serde_json::from_str(records[0].guardrail_response.get()).unwrap();
    assert!(message.contains("Blocked entity detected"));
    assert!(records[0].masked_entity_count.is_empty());
}

#[tokio::test]
async fn numbered_mode_stores_tokens_in_request_metadata() {
    let analyzer_mock = spawn_analyzer(
        |_| json_reply(r#"[{"entity_type":"PERSON","start":6,"end":11,"score":0.9}]"#),
        Duration::ZERO,
    )
    .await;
    let anonymizer_mock = spawn_anonymizer(
        |_| json_reply(r#"{"text":"IGNORED","items":[]}"#),
        Duration::ZERO,
    )
    .await;
    let analyzer = analyzer(&analyzer_mock.base);
    let client = anonymizer(&anonymizer_mock.base);
    let mut data = request_data("{}");
    let entities = no_entities();
    let text = client
        .check_pii(
            &analyzer,
            CheckPiiCall {
                text: "Hello Alice",
                output_parse_pii: true,
                presidio_config: None,
                dynamic_params: &NO_PARAMS,
                pii_entities_config: &entities,
                request_data: &mut data,
            },
        )
        .await
        .unwrap();
    assert_eq!(text, "Hello <PERSON_1>");
    assert_eq!(
        tokens(&data).get("<PERSON_1>").map(String::as_str),
        Some("Alice")
    );
    let records = logged_records(&data);
    assert_eq!(records[0].guardrail_status, "success");
    assert_eq!(records[0].masked_entity_count.get("PERSON"), Some(&1));
}

#[tokio::test]
async fn an_analyzer_failure_under_a_pii_config_is_recorded_and_returned() {
    let analyzer_mock = spawn_analyzer(
        |_| json_reply_with(StatusCode::INTERNAL_SERVER_ERROR, "boom"),
        Duration::ZERO,
    )
    .await;
    let anonymizer_mock =
        spawn_anonymizer(|_| json_reply(r#"{"text":"unused"}"#), Duration::ZERO).await;
    let mut analyzer = analyzer(&analyzer_mock.base);
    analyzer.settings.output_parse_pii = true;
    let client = anonymizer(&anonymizer_mock.base);
    let mut data = request_data("{}");
    let entities = no_entities();
    let error = client
        .check_pii(
            &analyzer,
            CheckPiiCall {
                text: "Alice",
                output_parse_pii: true,
                presidio_config: None,
                dynamic_params: &NO_PARAMS,
                pii_entities_config: &entities,
                request_data: &mut data,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, PresidioActionsError::Analysis { .. }));
    assert!(anonymizer_mock.requests.lock().unwrap().is_empty());
    let records = logged_records(&data);
    assert_eq!(records[0].guardrail_status, "guardrail_failed_to_respond");
    let message: String = serde_json::from_str(records[0].guardrail_response.get()).unwrap();
    assert!(message.contains("Presidio PII analysis failed"));
}

#[tokio::test]
async fn blank_text_skips_both_services_and_records_an_empty_response() {
    let analyzer_mock = spawn_analyzer(|_| json_reply("[]"), Duration::ZERO).await;
    let anonymizer_mock =
        spawn_anonymizer(|_| json_reply(r#"{"text":"unused"}"#), Duration::ZERO).await;
    let analyzer = analyzer(&analyzer_mock.base);
    let client = anonymizer(&anonymizer_mock.base);
    let mut data = request_data("{}");
    let entities = no_entities();
    let text = client
        .check_pii(
            &analyzer,
            CheckPiiCall {
                text: "   ",
                output_parse_pii: false,
                presidio_config: None,
                dynamic_params: &NO_PARAMS,
                pii_entities_config: &entities,
                request_data: &mut data,
            },
        )
        .await
        .unwrap();
    assert_eq!(text, "   ");
    assert!(analyzer_mock.requests.lock().unwrap().is_empty());
    assert!(anonymizer_mock.requests.lock().unwrap().is_empty());
    let records = logged_records(&data);
    assert_eq!(records[0].guardrail_status, "success");
    assert_eq!(records[0].guardrail_response.get(), "[]");
}

#[tokio::test]
async fn a_litellm_metadata_bucket_receives_the_log_record() {
    let analyzer_mock = spawn_analyzer(
        |_| json_reply(r#"[{"entity_type":"PERSON","start":6,"end":11,"score":0.9}]"#),
        Duration::ZERO,
    )
    .await;
    let anonymizer_mock = spawn_anonymizer(
        |_| json_reply(r#"{"text":"IGNORED","items":[]}"#),
        Duration::ZERO,
    )
    .await;
    let analyzer = analyzer(&analyzer_mock.base);
    let client = anonymizer(&anonymizer_mock.base);
    let mut data = request_data(r#"{"metadata":{},"litellm_metadata":{"existing_key":"kept"}}"#);
    let entities = no_entities();
    client
        .check_pii(
            &analyzer,
            CheckPiiCall {
                text: "Hello Alice",
                output_parse_pii: true,
                presidio_config: None,
                dynamic_params: &NO_PARAMS,
                pii_entities_config: &entities,
                request_data: &mut data,
            },
        )
        .await
        .unwrap();
    assert!(data.get("metadata").unwrap().get().contains("pii_tokens"));
    let bucket = data.get("litellm_metadata").unwrap().get();
    assert!(bucket.contains("standard_logging_guardrail_information"));
    assert!(bucket.contains("existing_key"));
    assert!(
        !data
            .get("metadata")
            .unwrap()
            .get()
            .contains("standard_logging")
    );
}

#[tokio::test]
async fn the_per_request_language_reaches_the_analyzer() {
    let analyzer_mock = spawn_analyzer(|_| json_reply("[]"), Duration::ZERO).await;
    let anonymizer_mock =
        spawn_anonymizer(|_| json_reply(r#"{"text":"unused"}"#), Duration::ZERO).await;
    let mut analyzer = analyzer(&analyzer_mock.base);
    analyzer.settings.pii_entities = vec!["PERSON".to_owned()];
    let client = anonymizer(&anonymizer_mock.base);
    let mut data = request_data("{}");
    let entities = no_entities();
    let presidio_config = PresidioPerRequestConfig {
        language: Some("fr".to_owned()),
        entities: None,
    };
    client
        .check_pii(
            &analyzer,
            CheckPiiCall {
                text: "Bonjour",
                output_parse_pii: false,
                presidio_config: Some(&presidio_config),
                dynamic_params: &NO_PARAMS,
                pii_entities_config: &entities,
                request_data: &mut data,
            },
        )
        .await
        .unwrap();
    let seen = analyze_requests(&analyzer_mock);
    assert_eq!(seen[0].language.as_deref(), Some("fr"));
    assert_eq!(seen[0].entities, Some(vec!["PERSON".to_owned()]));
}

#[tokio::test]
async fn dynamic_body_params_reach_the_analyzer_payload() {
    let analyzer_mock = spawn_analyzer(|_| json_reply("[]"), Duration::ZERO).await;
    let anonymizer_mock =
        spawn_anonymizer(|_| json_reply(r#"{"text":"unused"}"#), Duration::ZERO).await;
    let analyzer = analyzer(&analyzer_mock.base);
    let client = anonymizer(&anonymizer_mock.base);
    let mut data = request_data("{}");
    let entities = no_entities();
    let dynamic = BTreeMap::from([("language".to_owned(), GuardrailValue::Text("it".to_owned()))]);
    client
        .check_pii(
            &analyzer,
            CheckPiiCall {
                text: "Ciao",
                output_parse_pii: false,
                presidio_config: None,
                dynamic_params: &dynamic,
                pii_entities_config: &entities,
                request_data: &mut data,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        analyze_requests(&analyzer_mock)[0].language.as_deref(),
        Some("it")
    );
}
