use super::support::{
    guardrail, mock, mock_scripted, model_response, params, payload, sent_texts, violation,
};

#[tokio::test]
async fn scans_string_content() {
    let server = mock(200, payload(&[("Hate", 0)])).await;
    let hook = guardrail(&server.base, params(None, None));
    let response = model_response(r#"{"choices":[{"message":{"content":"bad"}}]}"#);
    hook.async_post_call_success_hook(Some(&response))
        .await
        .unwrap();
    assert_eq!(
        sent_texts(&server.seen.lock().unwrap()),
        vec!["bad".to_owned()]
    );
}

#[tokio::test]
async fn scans_every_choice_in_order() {
    let server = mock_scripted(vec![
        (200, payload(&[("Hate", 0)])),
        (200, payload(&[("Hate", 0)])),
    ])
    .await;
    let hook = guardrail(&server.base, params(None, None));
    let response = model_response(
        r#"{"choices":[{"message":{"content":"one"}},{"message":{"content":"two"}}]}"#,
    );
    hook.async_post_call_success_hook(Some(&response))
        .await
        .unwrap();
    assert_eq!(
        sent_texts(&server.seen.lock().unwrap()),
        vec!["one".to_owned(), "two".to_owned()]
    );
}

#[tokio::test]
async fn joins_text_blocks_with_a_newline() {
    let server = mock(200, payload(&[("Hate", 0)])).await;
    let hook = guardrail(&server.base, params(None, None));
    let response = model_response(
        r#"{"choices":[{"message":{"content":[
            {"type":"text","text":"first"},{"type":"text","text":"second"}]}}]}"#,
    );
    hook.async_post_call_success_hook(Some(&response))
        .await
        .unwrap();
    assert_eq!(
        sent_texts(&server.seen.lock().unwrap()),
        vec!["first\nsecond".to_owned()]
    );
}

#[tokio::test]
async fn skips_non_text_blocks_and_empty_text() {
    let server = mock(200, payload(&[("Hate", 0)])).await;
    let hook = guardrail(&server.base, params(None, None));
    let response = model_response(
        r#"{"choices":[{"message":{"content":[
            {"type":"image_url","image_url":{"url":"https://example.test"}},
            {"type":"text","text":""},{"type":"text","text":"kept"},
            {"type":"text","text":5}]}}]}"#,
    );
    hook.async_post_call_success_hook(Some(&response))
        .await
        .unwrap();
    assert_eq!(
        sent_texts(&server.seen.lock().unwrap()),
        vec!["kept".to_owned()]
    );
}

#[tokio::test]
async fn skips_choices_without_a_message_or_content() {
    let server = mock(200, payload(&[("Hate", 0)])).await;
    let hook = guardrail(&server.base, params(None, None));
    let response = model_response(r#"{"choices":[{}, {"message":{}}]}"#);
    hook.async_post_call_success_hook(Some(&response))
        .await
        .unwrap();
    assert!(server.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn skips_content_that_is_neither_text_nor_a_list() {
    let server = mock(200, payload(&[("Hate", 0)])).await;
    let hook = guardrail(&server.base, params(None, None));
    let response = model_response(r#"{"choices":[{"message":{"content":{"foo":1}}}]}"#);
    hook.async_post_call_success_hook(Some(&response))
        .await
        .unwrap();
    assert!(server.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn skips_a_response_that_is_not_a_model_response() {
    let server = mock(200, payload(&[("Hate", 0)])).await;
    let hook = guardrail(&server.base, params(None, None));
    hook.async_post_call_success_hook(None).await.unwrap();
    assert!(server.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn blocks_a_violating_choice() {
    let server = mock(200, payload(&[("Hate", 4)])).await;
    let hook = guardrail(&server.base, params(None, None));
    let response = model_response(r#"{"choices":[{"message":{"content":"bad"}}]}"#);
    let error = hook
        .async_post_call_success_hook(Some(&response))
        .await
        .unwrap_err();
    assert_eq!(violation(error), ("Hate".to_owned(), 2, 4));
}

#[tokio::test]
async fn stops_at_the_first_violating_choice() {
    let server = mock_scripted(vec![
        (200, payload(&[("Hate", 0)])),
        (200, payload(&[("Hate", 2)])),
        (200, payload(&[("Hate", 0)])),
    ])
    .await;
    let hook = guardrail(&server.base, params(None, None));
    let response = model_response(
        r#"{"choices":[{"message":{"content":"one"}},{"message":{"content":"two"}},
            {"message":{"content":"three"}}]}"#,
    );
    let error = hook
        .async_post_call_success_hook(Some(&response))
        .await
        .unwrap_err();
    assert_eq!(violation(error), ("Hate".to_owned(), 2, 2));
    assert_eq!(server.seen.lock().unwrap().len(), 2);
}
