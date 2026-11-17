#[tokio::test]
async fn route_streams_the_answer_for_an_admin_caller() {
    let model = FakeModel::new().replying(answer_reply("Hi"));
    let factory = FakeFactory::new(model.clone(), FakeSource::new());
    let request = with_principal(
        post(
            r#"{"messages": [{"role": "user", "content": "How much did we spend?"}], "model": "gpt-4o-mini"}"#,
        ),
        admin_principal("ana"),
    );
    let response = send(state(factory, None, FakeCallerId::from_subject()), request).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_text(response).await, answer_body());
    let calls = model.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "gpt-4o-mini");
    assert_eq!(calls[0].tools, 3);
    assert_eq!(calls[0].messages.len(), 2);
    assert_eq!(
        calls[0].messages[1],
        ChatMessage::User {
            content: "How much did we spend?".to_owned(),
        }
    );
}

#[tokio::test]
async fn route_uses_the_default_model_when_the_body_has_none() {
    let model = FakeModel::new().replying(answer_reply("Hi"));
    let factory = FakeFactory::new(model.clone(), FakeSource::new());
    let request = with_principal(
        post(r#"{"messages": [{"role": "user", "content": "hi"}]}"#),
        admin_principal("ana"),
    );
    let response = send(state(factory, None, FakeCallerId::from_subject()), request).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(model.calls()[0].model, "gpt-4o-mini");
}

#[tokio::test]
async fn route_rejects_a_missing_key() {
    let model = FakeModel::new().replying(answer_reply("Hi"));
    let factory = FakeFactory::new(model.clone(), FakeSource::new());
    let response = send(
        state(factory, None, FakeCallerId::from_subject()),
        post(r#"{"messages": []}"#),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        parsed(&body_text(response).await),
        json!({"error": {"message": "the API key is not valid", "type": "auth_error", "param": "None", "code": "401"}})
    );
    assert!(model.calls().is_empty());
}
