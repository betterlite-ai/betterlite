#![cfg(test)]

use std::time::Duration;

use super::shared::{NO_OVERRIDES, NO_PARAMS, analyzer, json_reply, settings, spawn_mock};
use crate::presidio_analysis::wire::{
    AnalyzeBody, AnalyzeOutcome, MockRedactedItem, MockRedactedText,
};
use crate::presidio_analysis::{RequestOverrides, json_body};

const EMAIL_BODY: &str = r#"[{"entity_type":"EMAIL_ADDRESS","start":3,"end":12,"score":0.9,"recognition_metadata":{"recognizer_name":"EmailRecognizer"}}]"#;

#[tokio::test]
async fn analyze_success_posts_the_payload_and_returns_the_items() {
    let mock = spawn_mock(|_| json_reply(EMAIL_BODY), Duration::ZERO).await;
    let outcome = analyzer(settings(&mock.base))
        .analyze_text("a: bob@example.com", &NO_OVERRIDES)
        .await
        .unwrap();
    let AnalyzeOutcome::Items(items) = outcome else {
        panic!("expected analyzer items");
    };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].entity_type.as_deref(), Some("EMAIL_ADDRESS"));
    assert_eq!((items[0].start, items[0].end), (Some(3), Some(12)));
    assert!(items[0].other.contains_key("recognition_metadata"));
    let seen = mock.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].text, "a: bob@example.com");
    assert_eq!(seen[0].language.as_deref(), Some("en"));
}

#[tokio::test]
async fn analyze_success_skips_list_entries_that_are_not_items() {
    let mock = spawn_mock(
        |_| {
            json_reply(
                r#"[1,{"entity_type":"BAD","start":"zero"},{"entity_type":"PERSON","start":0,"end":4,"score":0.5}]"#,
            )
        },
        Duration::ZERO,
    )
    .await;
    let AnalyzeOutcome::Items(items) = analyzer(settings(&mock.base))
        .analyze_text("john", &NO_OVERRIDES)
        .await
        .unwrap()
    else {
        panic!("expected analyzer items");
    };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].entity_type.as_deref(), Some("PERSON"));
}

#[tokio::test]
async fn analyze_success_takes_a_lone_dict_as_one_item() {
    let mock = spawn_mock(
        |_| json_reply(r#"{"entity_type":"PERSON","start":0,"end":4,"score":0.5}"#),
        Duration::ZERO,
    )
    .await;
    let AnalyzeOutcome::Items(items) = analyzer(settings(&mock.base))
        .analyze_text("john", &NO_OVERRIDES)
        .await
        .unwrap()
    else {
        panic!("expected analyzer items");
    };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].end, Some(4));
}

#[tokio::test]
async fn analyze_success_returns_an_empty_list_for_an_empty_analyzer_answer() {
    let mock = spawn_mock(|_| json_reply("[]"), Duration::ZERO).await;
    let AnalyzeOutcome::Items(items) = analyzer(settings(&mock.base))
        .analyze_text("nothing to find", &NO_OVERRIDES)
        .await
        .unwrap()
    else {
        panic!("expected analyzer items");
    };
    assert!(items.is_empty());
}

#[tokio::test]
async fn analyze_success_sends_the_request_language_over_the_configured_one() {
    let mock = spawn_mock(|_| json_reply("[]"), Duration::ZERO).await;
    let request = RequestOverrides {
        language: Some("de"),
        dynamic_params: &NO_PARAMS,
    };
    analyzer(settings(&mock.base))
        .analyze_text("hallo", &request)
        .await
        .unwrap();
    assert_eq!(mock.seen.lock().unwrap()[0].language.as_deref(), Some("de"));
}

#[tokio::test]
async fn analyze_success_returns_the_mock_redacted_text_without_calling_the_analyzer() {
    let mock = spawn_mock(|_| json_reply("[]"), Duration::ZERO).await;
    let redacted = MockRedactedText {
        text: "<PERSON>".to_owned(),
        items: vec![MockRedactedItem {
            entity_type: Some("PERSON".to_owned()),
        }],
    };
    let mut configured = settings(&mock.base);
    configured.mock_redacted_text = Some(redacted.clone());
    let outcome = analyzer(configured)
        .analyze_text("john", &NO_OVERRIDES)
        .await
        .unwrap();
    assert!(matches!(outcome, AnalyzeOutcome::Redacted(found) if found == redacted));
    assert!(mock.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn analyze_success_json_body_reads_the_response_as_an_analyzer_body() {
    let mock = spawn_mock(|_| json_reply(EMAIL_BODY), Duration::ZERO).await;
    let response = reqwest::Client::default()
        .post(format!("{}/analyze", mock.base))
        .body(r#"{"text":"x"}"#)
        .send()
        .await
        .unwrap();
    let body = json_body(response).await.unwrap();
    assert!(matches!(body, AnalyzeBody::List(entries) if entries.len() == 1));
}

#[tokio::test]
async fn analyze_success_passes_unknown_analyzer_fields_through_unchanged() {
    let mock = spawn_mock(
        |_| {
            json_reply(
                r#"[{"entity_type":"PERSON","start":0,"end":4,"score":0.85,"analysis_explanation":{"original_score":0.85,"textual_explanation":null}}]"#,
            )
        },
        Duration::ZERO,
    )
    .await;
    let AnalyzeOutcome::Items(items) = analyzer(settings(&mock.base))
        .analyze_text("john", &NO_OVERRIDES)
        .await
        .unwrap()
    else {
        panic!("expected analyzer items");
    };
    assert_eq!(items[0].score, Some(0.85_f64));
    assert_eq!(
        serde_json::to_string(&items[0]).unwrap(),
        r#"{"entity_type":"PERSON","start":0,"end":4,"score":0.85,"analysis_explanation":{"original_score":0.85,"textual_explanation":null}}"#
    );
}

#[tokio::test]
async fn analyze_success_sends_the_configured_entities_only_when_there_are_some() {
    let mock = spawn_mock(|_| json_reply("[]"), Duration::ZERO).await;
    analyzer(settings(&mock.base))
        .analyze_text("john", &NO_OVERRIDES)
        .await
        .unwrap();
    let mut configured = settings(&mock.base);
    configured.pii_entities = vec!["PERSON".to_owned()];
    analyzer(configured)
        .analyze_text("jane", &NO_OVERRIDES)
        .await
        .unwrap();
    let entities: Vec<Option<Vec<String>>> = mock
        .seen
        .lock()
        .unwrap()
        .iter()
        .map(|request| request.entities.clone())
        .collect();
    assert_eq!(entities, vec![None, Some(vec!["PERSON".to_owned()])]);
}
