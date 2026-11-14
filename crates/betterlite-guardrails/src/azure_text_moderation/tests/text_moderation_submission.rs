use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use reqwest::StatusCode;

use super::support::{
    analysis, guardrail, inputs, mock, mock_scripted, params, payload, sent_texts, violation,
};
use crate::azure_content_safety::legacy_wire::TextCategory;
use crate::azure_text_moderation::config::OptionalParams;
use crate::azure_text_moderation::error::AzureTextModerationError;
use crate::azure_text_moderation::wire::OutputType;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;

const DEFAULT_BODY: &str = r#"{"text":"hello","categories":["Hate","Sexual","SelfHarm","Violence"],"blocklistNames":null,"haltOnBlocklistHit":false,"outputType":"FourSeverityLevels"}"#;

#[tokio::test]
async fn posts_a_short_text_to_the_analyze_endpoint() {
    let server = mock(200, payload(&[("Hate", 0)])).await;
    let hook = guardrail(&server.base, OptionalParams::default());
    let answer = hook.async_make_request("hello").await.unwrap();
    assert_eq!(answer, analysis(&[("Hate", 0)]));
    let seen = server.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    let request = seen.first().unwrap();
    assert_eq!(request.path, "/contentsafety/text:analyze");
    assert_eq!(request.query.as_deref(), Some("api-version=2024-09-01"));
    assert_eq!(request.body, DEFAULT_BODY);
}

#[tokio::test]
async fn posts_the_configured_request_body_options() {
    let server = mock(200, payload(&[("Hate", 0)])).await;
    let optional_params = OptionalParams {
        severity_threshold: None,
        severity_threshold_by_category: None,
        categories: Some(vec![TextCategory::Violence]),
        blocklist_names: Some(vec!["list-a".to_owned()]),
        halt_on_blocklist_hit: Some(true),
        output_type: Some(OutputType::EightSeverityLevels),
    };
    let hook = guardrail(&server.base, optional_params);
    hook.async_make_request("hello").await.unwrap();
    let seen = server.seen.lock().unwrap().clone();
    assert_eq!(
        seen.first().unwrap().body,
        r#"{"text":"hello","categories":["Violence"],"blocklistNames":["list-a"],"haltOnBlocklistHit":true,"outputType":"EightSeverityLevels"}"#
    );
}

#[tokio::test]
async fn parses_a_response_that_reports_blocklist_matches() {
    let server = mock(
        200,
        r#"{"blocklistsMatch":[{"blocklistName":"list-a","blocklistItemId":"1","blocklistItemText":"match"}],"categoriesAnalysis":[{"category":"Hate","severity":0}]}"#.to_owned(),
    )
    .await;
    let hook = guardrail(&server.base, OptionalParams::default());
    let answer = hook.async_make_request("hello").await.unwrap();
    assert_eq!(answer, analysis(&[("Hate", 0)]));
}

#[tokio::test]
async fn splits_a_long_text_and_returns_the_last_response() {
    let text = "x".repeat(10_001);
    let server = mock_scripted(vec![
        (200, payload(&[("Hate", 0)])),
        (200, payload(&[("Hate", 1)])),
    ])
    .await;
    let hook = guardrail(&server.base, OptionalParams::default());
    let answer = hook.async_make_request(&text).await.unwrap();
    assert_eq!(answer, analysis(&[("Hate", 1)]));
    let sent = sent_texts(&server.seen.lock().unwrap());
    assert_eq!(sent.len(), 2);
    assert_eq!(sent.first().unwrap().len(), 10_000);
    assert_eq!(sent.get(1).unwrap().len(), 1);
}

#[tokio::test]
async fn checks_every_chunk_before_submitting_the_next() {
    let text = "x".repeat(10_001);
    let server = mock_scripted(vec![
        (200, payload(&[("Hate", 2)])),
        (200, payload(&[("Hate", 0)])),
    ])
    .await;
    let hook = guardrail(&server.base, OptionalParams::default());
    let error = hook.async_make_request(&text).await.unwrap_err();
    assert_eq!(violation(error), ("Hate".to_owned(), 2, 2));
    assert_eq!(server.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn checks_the_last_chunk_too() {
    let text = "x".repeat(10_001);
    let server = mock_scripted(vec![
        (200, payload(&[("Hate", 0)])),
        (200, payload(&[("Hate", 3)])),
    ])
    .await;
    let hook = guardrail(&server.base, OptionalParams::default());
    let error = hook.async_make_request(&text).await.unwrap_err();
    assert_eq!(violation(error), ("Hate".to_owned(), 2, 3));
    assert_eq!(server.seen.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn propagates_an_api_error() {
    let server = mock(500, "{}".to_owned()).await;
    let hook = guardrail(&server.base, OptionalParams::default());
    let error = hook.async_make_request("hello").await.unwrap_err();
    assert!(matches!(error, AzureTextModerationError::ContentSafety(_)));
    assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn apply_guardrail_scans_every_non_empty_text() {
    let server = mock_scripted(vec![
        (200, payload(&[("Hate", 0)])),
        (200, payload(&[("Hate", 0)])),
    ])
    .await;
    let hook = guardrail(&server.base, params(None, None));
    let texts = vec!["first".to_owned(), String::new(), "second".to_owned()];
    let mut request_data = BTreeMap::new();
    let returned = hook
        .apply_guardrail(
            inputs(Some(texts.clone()), None),
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap();
    assert_eq!(returned.texts, Some(texts));
    assert_eq!(
        sent_texts(&server.seen.lock().unwrap()),
        vec!["first".to_owned(), "second".to_owned()]
    );
}

#[tokio::test]
async fn apply_guardrail_leaves_a_request_without_texts_alone() {
    let server = mock(200, payload(&[("Hate", 0)])).await;
    let hook = guardrail(&server.base, params(None, None));
    let mut request_data = BTreeMap::new();
    let returned = hook
        .apply_guardrail(
            inputs(None, Some("gpt-4o")),
            &mut request_data,
            GuardrailInputType::Response,
            None,
        )
        .await
        .unwrap();
    assert_eq!(returned.texts, None);
    assert_eq!(returned.model.as_deref(), Some("gpt-4o"));
    assert!(server.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn apply_guardrail_maps_a_violation_to_the_litellm_detail() {
    let server = mock(200, payload(&[("Hate", 3)])).await;
    let hook = guardrail(&server.base, params(None, None));
    let mut request_data = BTreeMap::new();
    let error = hook
        .apply_guardrail(
            inputs(Some(vec!["bad".to_owned()]), None),
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap_err();
    let (status, detail) = match error {
        GuardrailFailure::HttpObject { status, detail } => (status, detail),
        other => panic!("expected an HttpObject failure, got {other:?}"),
    };
    assert_eq!(status, 400);
    assert_eq!(detail.len(), 1);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(
            "Azure Content Safety Guardrail: Hate crossed severity 2, Got severity: 3".to_owned()
        ))
    );
}

#[tokio::test]
async fn apply_guardrail_maps_a_transport_error_to_http() {
    let server = mock(503, "{}".to_owned()).await;
    let hook = guardrail(&server.base, params(None, None));
    let mut request_data = BTreeMap::new();
    let error = hook
        .apply_guardrail(
            inputs(Some(vec!["hello".to_owned()]), None),
            &mut request_data,
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap_err();
    let (status, detail) = match error {
        GuardrailFailure::Http { status, detail } => (status, detail),
        other => panic!("expected an Http failure, got {other:?}"),
    };
    assert_eq!(status, 503);
    assert!(detail.contains("503"));
}
