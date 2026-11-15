use super::support::{guardrail, mock, params, payload, sent_texts};
use crate::azure_text_moderation::error::AzureTextModerationError;

#[tokio::test]
async fn returns_a_clean_chunk_unchanged() {
    let server = mock(200, payload(&[("Hate", 1)])).await;
    let hook = guardrail(&server.base, params(None, None));
    let returned = hook.async_post_call_streaming_hook("hello").await.unwrap();
    assert_eq!(returned, "hello");
    assert_eq!(
        sent_texts(&server.seen.lock().unwrap()),
        vec!["hello".to_owned()]
    );
}

#[tokio::test]
async fn returns_an_empty_chunk_without_scanning() {
    let server = mock(200, payload(&[("Hate", 1)])).await;
    let hook = guardrail(&server.base, params(None, None));
    let returned = hook.async_post_call_streaming_hook("").await.unwrap();
    assert_eq!(returned, "");
    assert!(server.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn returns_an_error_frame_for_a_violation() {
    let server = mock(200, payload(&[("Hate", 2)])).await;
    let hook = guardrail(&server.base, params(None, None));
    let returned = hook.async_post_call_streaming_hook("bad").await.unwrap();
    assert_eq!(
        returned,
        "data: {\"error\":{\"error\":\"Azure Content Safety Guardrail: Hate crossed severity 2, Got severity: 2\"}}\n\n"
    );
}

#[tokio::test]
async fn names_the_configured_global_threshold_in_the_frame() {
    let server = mock(200, payload(&[("Hate", 1)])).await;
    let hook = guardrail(&server.base, params(Some(1), None));
    let returned = hook.async_post_call_streaming_hook("bad").await.unwrap();
    assert_eq!(
        returned,
        "data: {\"error\":{\"error\":\"Azure Content Safety Guardrail: Hate crossed severity 1, Got severity: 1\"}}\n\n"
    );
}

#[tokio::test]
async fn propagates_a_transport_error_instead_of_a_frame() {
    let server = mock(500, "{}".to_owned()).await;
    let hook = guardrail(&server.base, params(None, None));
    let error = hook
        .async_post_call_streaming_hook("bad")
        .await
        .unwrap_err();
    assert!(matches!(error, AzureTextModerationError::ContentSafety(_)));
}
