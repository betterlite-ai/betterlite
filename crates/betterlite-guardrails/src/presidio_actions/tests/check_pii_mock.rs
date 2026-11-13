#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::enums::PiiAction;

use super::shared::{
    NO_PARAMS, analyzer, anonymizer, json_reply, logged_records, request_data, spawn_analyzer,
    spawn_anonymizer,
};
use crate::presidio_actions::check::CheckPiiCall;
use crate::presidio_analysis::wire::{MockRedactedItem, MockRedactedText};

fn mock_text(text: &str) -> MockRedactedText {
    MockRedactedText {
        text: text.to_owned(),
        items: vec![MockRedactedItem {
            entity_type: Some("PERSON".to_owned()),
        }],
    }
}

#[tokio::test]
async fn the_mock_text_is_returned_without_calling_either_service() {
    let analyzer_mock = spawn_analyzer(|_| json_reply("[]"), Duration::ZERO).await;
    let anonymizer_mock =
        spawn_anonymizer(|_| json_reply(r#"{"text":"SERVER"}"#), Duration::ZERO).await;
    let mut analyzer = analyzer(&analyzer_mock.base);
    analyzer.settings.mock_redacted_text = Some(mock_text("MOCKED"));
    let client = anonymizer(&anonymizer_mock.base);
    let mut data = request_data("{}");
    let entities = BTreeMap::new();
    let text = client
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
        .unwrap();
    assert_eq!(text, "MOCKED");
    assert!(analyzer_mock.requests.lock().unwrap().is_empty());
    assert!(anonymizer_mock.requests.lock().unwrap().is_empty());
    let records = logged_records(&data);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].guardrail_status, "success");
    assert_eq!(records[0].guardrail_response.get(), "{}");
    assert!(records[0].masked_entity_count.is_empty());
}

#[tokio::test]
async fn an_empty_mock_text_is_returned_as_is() {
    let analyzer_mock = spawn_analyzer(|_| json_reply("[]"), Duration::ZERO).await;
    let anonymizer_mock =
        spawn_anonymizer(|_| json_reply(r#"{"text":"SERVER"}"#), Duration::ZERO).await;
    let mut analyzer = analyzer(&analyzer_mock.base);
    analyzer.settings.mock_redacted_text = Some(MockRedactedText {
        text: String::new(),
        items: Vec::new(),
    });
    let client = anonymizer(&anonymizer_mock.base);
    let mut data = request_data("{}");
    let entities = BTreeMap::new();
    let text = client
        .check_pii(
            &analyzer,
            CheckPiiCall {
                text: "Alice",
                output_parse_pii: false,
                presidio_config: None,
                dynamic_params: &NO_PARAMS,
                pii_entities_config: &entities,
                request_data: &mut data,
            },
        )
        .await
        .unwrap();
    assert!(text.is_empty());
}

#[tokio::test]
async fn the_mock_bypasses_blocked_entity_checks() {
    let analyzer_mock = spawn_analyzer(|_| json_reply("[]"), Duration::ZERO).await;
    let anonymizer_mock =
        spawn_anonymizer(|_| json_reply(r#"{"text":"SERVER"}"#), Duration::ZERO).await;
    let mut analyzer = analyzer(&analyzer_mock.base);
    analyzer.settings.mock_redacted_text = Some(mock_text("MOCKED"));
    let client = anonymizer(&anonymizer_mock.base);
    let mut data = request_data("{}");
    let entities = BTreeMap::from([("PERSON".to_owned(), PiiAction::Block)]);
    let text = client
        .check_pii(
            &analyzer,
            CheckPiiCall {
                text: "Alice",
                output_parse_pii: false,
                presidio_config: None,
                dynamic_params: &NO_PARAMS,
                pii_entities_config: &entities,
                request_data: &mut data,
            },
        )
        .await
        .unwrap();
    assert_eq!(text, "MOCKED");
}
