#[rstest]
#[case::missing_messages(
    "{}",
    json!({"detail": [{"type": "missing", "loc": ["body", "messages"], "msg": "Field required"}]})
)]
#[case::messages_text(
    r#"{"messages": "hi"}"#,
    json!({"detail": [{"type": "list_type", "loc": ["body", "messages"], "msg": "Input should be a valid list"}]})
)]
#[case::messages_null(
    r#"{"messages": null}"#,
    json!({"detail": [{"type": "list_type", "loc": ["body", "messages"], "msg": "Input should be a valid list"}]})
)]
#[case::item_text(
    r#"{"messages": ["hi"]}"#,
    json!({"detail": [{"type": "model_attributes_type", "loc": ["body", "messages", 0], "msg": "Input should be a valid dictionary or object to extract fields from"}]})
)]
#[case::missing_role(
    r#"{"messages": [{"content": "hi"}]}"#,
    json!({"detail": [{"type": "missing", "loc": ["body", "messages", 0, "role"], "msg": "Field required"}]})
)]
#[case::bad_role(
    r#"{"messages": [{"role": "system", "content": "hi"}]}"#,
    json!({"detail": [{"type": "literal_error", "loc": ["body", "messages", 0, "role"], "msg": "Input should be 'user' or 'assistant'"}]})
)]
#[case::role_int(
    r#"{"messages": [{"role": 5, "content": "hi"}]}"#,
    json!({"detail": [{"type": "literal_error", "loc": ["body", "messages", 0, "role"], "msg": "Input should be 'user' or 'assistant'"}]})
)]
#[case::role_null(
    r#"{"messages": [{"role": null, "content": "hi"}]}"#,
    json!({"detail": [{"type": "literal_error", "loc": ["body", "messages", 0, "role"], "msg": "Input should be 'user' or 'assistant'"}]})
)]
#[case::messages_object(
    r#"{"messages": {}}"#,
    json!({"detail": [{"type": "list_type", "loc": ["body", "messages"], "msg": "Input should be a valid list"}]})
)]
#[case::missing_content(
    r#"{"messages": [{"role": "user"}]}"#,
    json!({"detail": [{"type": "missing", "loc": ["body", "messages", 0, "content"], "msg": "Field required"}]})
)]
#[case::content_int(
    r#"{"messages": [{"role": "user", "content": 5}]}"#,
    json!({"detail": [{"type": "string_type", "loc": ["body", "messages", 0, "content"], "msg": "Input should be a valid string"}]})
)]
#[case::content_null(
    r#"{"messages": [{"role": "user", "content": null}]}"#,
    json!({"detail": [{"type": "string_type", "loc": ["body", "messages", 0, "content"], "msg": "Input should be a valid string"}]})
)]
#[case::model_int(
    r#"{"messages": [], "model": 5}"#,
    json!({"detail": [{"type": "string_type", "loc": ["body", "model"], "msg": "Input should be a valid string"}]})
)]
#[case::body_list(
    "[1]",
    json!({"detail": [{"type": "model_attributes_type", "loc": ["body"], "msg": "Input should be a valid dictionary or object to extract fields from"}]})
)]
#[case::empty(
    "",
    json!({"detail": [{"type": "missing", "loc": ["body"], "msg": "Field required"}]})
)]
#[case::two_messages(
    r#"{"messages": [{"role": "system", "content": "hi"}, {"role": "user", "content": 5}]}"#,
    json!({"detail": [
        {"type": "literal_error", "loc": ["body", "messages", 0, "role"], "msg": "Input should be 'user' or 'assistant'"},
        {"type": "string_type", "loc": ["body", "messages", 1, "content"], "msg": "Input should be a valid string"}
    ]})
)]
#[case::missing_role_and_content(
    r#"{"messages": [{}]}"#,
    json!({"detail": [
        {"type": "missing", "loc": ["body", "messages", 0, "role"], "msg": "Field required"},
        {"type": "missing", "loc": ["body", "messages", 0, "content"], "msg": "Field required"}
    ]})
)]
#[case::bad_role_and_model(
    r#"{"messages": [{"role": "nope", "content": "hi"}], "model": 5}"#,
    json!({"detail": [
        {"type": "literal_error", "loc": ["body", "messages", 0, "role"], "msg": "Input should be 'user' or 'assistant'"},
        {"type": "string_type", "loc": ["body", "model"], "msg": "Input should be a valid string"}
    ]})
)]
#[tokio::test]
async fn body_answers_the_fastapi_detail_list(#[case] raw: &str, #[case] expected: JsonValue) {
    let model = FakeModel::new().replying(answer_reply("Hi"));
    let factory = FakeFactory::new(model.clone(), FakeSource::new());
    let request = with_principal(post(raw), master_principal());
    let response = send(state(factory, None, FakeCallerId::from_subject()), request).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(parsed(&body_text(response).await), expected);
    assert!(model.calls().is_empty());
}

#[tokio::test]
async fn body_accepts_an_empty_message_list() {
    let model = FakeModel::new().replying(answer_reply("Hi"));
    let factory = FakeFactory::new(model.clone(), FakeSource::new());
    let request = with_principal(post(r#"{"messages": []}"#), master_principal());
    let response = send(state(factory, None, FakeCallerId::from_subject()), request).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_text(response).await, answer_body());
    assert_eq!(model.calls()[0].messages.len(), 1);
}

#[tokio::test]
async fn body_ignores_extra_keys_and_a_null_model() {
    let model = FakeModel::new().replying(answer_reply("Hi"));
    let factory = FakeFactory::new(model.clone(), FakeSource::new());
    let request = with_principal(
        post(
            r#"{"messages": [{"role": "assistant", "content": "hi", "name": "bot"}], "model": null, "stream": true}"#,
        ),
        master_principal(),
    );
    let response = send(state(factory, None, FakeCallerId::from_subject()), request).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_text(response).await, answer_body());
    assert_eq!(
        model.calls()[0].messages[1],
        ChatMessage::Assistant {
            content: "hi".to_owned(),
        }
    );
    assert_eq!(model.calls()[0].model, "gpt-4o-mini");
}

#[tokio::test]
async fn body_reports_invalid_json_with_fastapis_shape() {
    let model = FakeModel::new().replying(answer_reply("Hi"));
    let factory = FakeFactory::new(model.clone(), FakeSource::new());
    let request = with_principal(post("{"), master_principal());
    let response = send(state(factory, None, FakeCallerId::from_subject()), request).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        parsed(&body_text(response).await),
        json!({"detail": [{"type": "json_invalid", "loc": ["body"], "msg": "JSON decode error"}]})
    );
    assert!(model.calls().is_empty());
}

#[rstest]
#[case::infinity(r#"{"messages": [], "model": Infinity}"#)]
#[case::nan(r#"{"messages": [], "model": NaN}"#)]
#[tokio::test]
async fn body_rejects_non_finite_numbers_as_invalid_json(#[case] raw: &str) {
    let model = FakeModel::new().replying(answer_reply("Hi"));
    let factory = FakeFactory::new(model.clone(), FakeSource::new());
    let request = with_principal(post(raw), master_principal());
    let response = send(state(factory, None, FakeCallerId::from_subject()), request).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = parsed(&body_text(response).await);
    assert_eq!(body["detail"][0]["type"], "json_invalid");
    assert_eq!(body["detail"][0]["loc"], json!(["body"]));
    assert!(model.calls().is_empty());
}

#[tokio::test]
async fn body_reads_json_whatever_the_content_type() {
    let model = FakeModel::new().replying(answer_reply("Hi"));
    let factory = FakeFactory::new(model.clone(), FakeSource::new());
    let request = Request::builder()
        .method(Method::POST)
        .uri("/usage/ai/chat")
        .header("content-type", "text/plain")
        .body(Body::from(
            r#"{"messages": [{"role": "user", "content": "hi"}]}"#,
        ))
        .expect("the request builds");
    let response = send(
        state(factory, None, FakeCallerId::from_subject()),
        with_principal(request, master_principal()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_text(response).await, answer_body());
}

#[tokio::test]
async fn body_reports_invalid_utf8_the_way_starlette_does() {
    let model = FakeModel::new().replying(answer_reply("Hi"));
    let factory = FakeFactory::new(model.clone(), FakeSource::new());
    let request = Request::builder()
        .method(Method::POST)
        .uri("/usage/ai/chat")
        .header("content-type", "application/json")
        .body(Body::from(vec![0xff_u8, 0xfe]))
        .expect("the request builds");
    let response = send(
        state(factory, None, FakeCallerId::from_subject()),
        with_principal(request, master_principal()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        parsed(&body_text(response).await),
        json!({"detail": "There was an error parsing the body"})
    );
    assert!(model.calls().is_empty());
}
