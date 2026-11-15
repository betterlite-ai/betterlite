use super::support::{data, guardrail, mock, params, payload, sent_texts, violation};
use crate::azure_text_moderation::error::AzureTextModerationError;

#[tokio::test]
async fn scans_the_last_user_message_of_a_chat_request() {
    let server = mock(200, payload(&[("Hate", 0)])).await;
    let hook = guardrail(&server.base, params(None, None));
    let request = data(
        r#"{"messages":[{"role":"user","content":"first"},
            {"role":"assistant","content":"ok"},{"role":"user","content":"bad"}]}"#,
    );
    let returned = hook
        .async_pre_call_hook(&request, "acompletion")
        .await
        .unwrap();
    assert_eq!(returned, None);
    assert_eq!(
        sent_texts(&server.seen.lock().unwrap()),
        vec!["bad".to_owned()]
    );
}

#[tokio::test]
async fn returns_the_request_when_messages_are_absent() {
    let server = mock(200, payload(&[("Hate", 0)])).await;
    let hook = guardrail(&server.base, params(None, None));
    let request = data(r#"{"model":"gpt-4o"}"#);
    let returned = hook
        .async_pre_call_hook(&request, "completion")
        .await
        .unwrap();
    assert_eq!(returned, Some(request));
    assert!(server.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn returns_the_request_when_messages_are_null() {
    let server = mock(200, payload(&[("Hate", 0)])).await;
    let hook = guardrail(&server.base, params(None, None));
    let request = data(r#"{"messages":null}"#);
    let returned = hook
        .async_pre_call_hook(&request, "completion")
        .await
        .unwrap();
    assert_eq!(returned, Some(request));
    assert!(server.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn scans_a_responses_string_input() {
    let server = mock(200, payload(&[("Hate", 0)])).await;
    let hook = guardrail(&server.base, params(None, None));
    let request = data(r#"{"input":"bad"}"#);
    let returned = hook
        .async_pre_call_hook(&request, "responses")
        .await
        .unwrap();
    assert_eq!(returned, None);
    assert_eq!(
        sent_texts(&server.seen.lock().unwrap()),
        vec!["bad".to_owned()]
    );
}

#[tokio::test]
async fn scans_the_last_user_message_of_a_responses_input() {
    let server = mock(200, payload(&[("Hate", 0)])).await;
    let hook = guardrail(&server.base, params(None, None));
    let request = data(
        r#"{"input":[{"role":"user","content":"first"},
            {"role":"assistant","content":"ok"},{"role":"user","content":"bad"}]}"#,
    );
    let returned = hook
        .async_pre_call_hook(&request, "aresponses")
        .await
        .unwrap();
    assert_eq!(returned, None);
    assert_eq!(
        sent_texts(&server.seen.lock().unwrap()),
        vec!["bad".to_owned()]
    );
}

#[tokio::test]
async fn skips_a_responses_input_that_is_not_text_or_a_list() {
    let server = mock(200, payload(&[("Hate", 0)])).await;
    let hook = guardrail(&server.base, params(None, None));
    let request = data(r#"{"input":{"text":"bad"}}"#);
    let returned = hook
        .async_pre_call_hook(&request, "responses")
        .await
        .unwrap();
    assert_eq!(returned, None);
    assert!(server.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn skips_a_chat_request_without_a_user_message() {
    let server = mock(200, payload(&[("Hate", 0)])).await;
    let hook = guardrail(&server.base, params(None, None));
    let request = data(r#"{"messages":[{"role":"assistant","content":"hi"}]}"#);
    let returned = hook
        .async_pre_call_hook(&request, "completion")
        .await
        .unwrap();
    assert_eq!(returned, None);
    assert!(server.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn blocks_a_violating_prompt() {
    let server = mock(200, payload(&[("Hate", 3)])).await;
    let hook = guardrail(&server.base, params(None, None));
    let request = data(r#"{"messages":[{"role":"user","content":"bad"}]}"#);
    let error = hook
        .async_pre_call_hook(&request, "completion")
        .await
        .unwrap_err();
    assert_eq!(violation(error), ("Hate".to_owned(), 2, 3));
    assert_eq!(server.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn propagates_a_transport_error() {
    let server = mock(500, "{}".to_owned()).await;
    let hook = guardrail(&server.base, params(None, None));
    let request = data(r#"{"messages":[{"role":"user","content":"bad"}]}"#);
    let error = hook
        .async_pre_call_hook(&request, "completion")
        .await
        .unwrap_err();
    assert!(matches!(error, AzureTextModerationError::ContentSafety(_)));
}
