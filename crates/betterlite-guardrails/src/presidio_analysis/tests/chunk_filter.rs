#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use axum::http::StatusCode;

use super::shared::{
    NO_OVERRIDES, Trigger, analyzer, closed_settings, json_reply, settings, spawn_mock,
    text_with_secret_at_33,
};
use crate::presidio_analysis::error::PresidioAnalysisError;
use crate::presidio_analysis::wire::AnalyzeOutcome;

#[tokio::test]
async fn chunk_filter_drops_a_weak_long_span_before_it_can_win_the_merge() {
    // The chunk at offset 20 reports a long weak span, the chunk at offset 30 a short strong one.
    let mock = spawn_mock(
        |request| match request.text.find("SECRET") {
            Some(start) if start >= 10 => json_reply(&format!(
                r#"[{{"entity_type":"PERSON","start":{},"end":{},"score":0.1}}]"#,
                start - 2,
                start + 8
            )),
            Some(start) => json_reply(&format!(
                r#"[{{"entity_type":"PERSON","start":{start},"end":{},"score":0.9}}]"#,
                start.saturating_add(6)
            )),
            None => json_reply("[]"),
        },
        Duration::ZERO,
    )
    .await;
    let mut configured = settings(&mock.base);
    configured.chunk_size_bytes = 20;
    configured.score_thresholds = BTreeMap::from([("ALL".to_owned(), 0.5_f64)]);
    let AnalyzeOutcome::Items(items) = analyzer(configured)
        .analyze_text(&text_with_secret_at_33(), &NO_OVERRIDES)
        .await
        .unwrap()
    else {
        panic!("expected analyzer items");
    };
    assert_eq!(items.len(), 1);
    assert_eq!((items[0].start, items[0].end), (Some(33), Some(39)));
    assert_eq!(items[0].score, Some(0.9_f64));
}

#[tokio::test]
async fn chunk_filter_applies_the_deny_list_to_every_chunk() {
    let mock = spawn_mock(
        |_| {
            json_reply(
                r#"[{"entity_type":"PERSON","start":0,"end":2,"score":0.9},{"entity_type":"EMAIL_ADDRESS","start":3,"end":6,"score":0.9}]"#,
            )
        },
        Duration::ZERO,
    )
    .await;
    let mut configured = settings(&mock.base);
    configured.chunk_size_bytes = 20;
    configured.entities_deny_list = vec!["PERSON".to_owned()];
    let AnalyzeOutcome::Items(items) = analyzer(configured)
        .analyze_text(&text_with_secret_at_33(), &NO_OVERRIDES)
        .await
        .unwrap()
    else {
        panic!("expected analyzer items");
    };
    assert!(!items.is_empty());
    assert!(
        items
            .iter()
            .all(|item| item.entity_type.as_deref() == Some("EMAIL_ADDRESS"))
    );
}

#[tokio::test]
async fn chunk_filter_raises_when_any_chunk_fails() {
    let mock = spawn_mock(
        |request| {
            if request.text.contains("SECRET") {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "text/plain",
                    "down".to_owned(),
                )
            } else {
                json_reply("[]")
            }
        },
        Duration::ZERO,
    )
    .await;
    let mut configured = closed_settings(&mock.base, Trigger::PiiEntities);
    configured.chunk_size_bytes = 20;
    let error = analyzer(configured)
        .analyze_text(&text_with_secret_at_33(), &NO_OVERRIDES)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        PresidioAnalysisError::GuardrailRaised { .. }
    ));
}
