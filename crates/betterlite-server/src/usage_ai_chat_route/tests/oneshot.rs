#[tokio::test]
async fn oneshot_answers_the_usage_chat_path() {
    let model = FakeModel::new().replying(answer_reply("Hi"));
    let factory = FakeFactory::new(model, FakeSource::new());
    let request = with_principal(post(STREAM_BODY), master_principal());
    let response = send(state(factory, None, FakeCallerId::from_subject()), request).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_text(response).await, answer_body());
}

#[tokio::test]
async fn oneshot_leaves_the_v1_alias_unrouted() {
    let model = FakeModel::new().replying(answer_reply("Hi"));
    let factory = FakeFactory::new(model, FakeSource::new());
    let request = Request::builder()
        .method(Method::POST)
        .uri("/v1/usage/ai/chat")
        .header("content-type", "application/json")
        .body(Body::from(STREAM_BODY))
        .expect("the request builds");
    let response = send(
        state(factory, None, FakeCallerId::from_subject()),
        with_principal(request, master_principal()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn oneshot_rejects_a_get() {
    let model = FakeModel::new().replying(answer_reply("Hi"));
    let factory = FakeFactory::new(model, FakeSource::new());
    let request = Request::builder()
        .method(Method::GET)
        .uri("/usage/ai/chat")
        .body(Body::empty())
        .expect("the request builds");
    let response = send(
        state(factory, None, FakeCallerId::from_subject()),
        with_principal(request, master_principal()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn oneshot_returns_401_before_a_malformed_body() {
    let model = FakeModel::new().replying(answer_reply("Hi"));
    let factory = FakeFactory::new(model, FakeSource::new());
    let response = send(
        state(factory, None, FakeCallerId::from_subject()),
        post("{"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        parsed(&body_text(response).await),
        json!({"error": {"message": "the API key is not valid", "type": "auth_error", "param": "None", "code": "401"}})
    );
}
