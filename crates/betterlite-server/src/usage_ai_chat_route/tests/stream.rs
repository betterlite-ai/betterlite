const STREAM_BODY: &str = r#"{"messages": [{"role": "user", "content": "hi"}]}"#;

#[tokio::test]
async fn stream_passes_the_engine_frames_through_in_order() {
    let model = FakeModel::new().replying(answer_reply("Hi"));
    let factory = FakeFactory::new(model, FakeSource::new());
    let request = with_principal(post(STREAM_BODY), master_principal());
    let response = send(state(factory, None, FakeCallerId::from_subject()), request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut frames = frames(response);
    assert_eq!(
        next_frame(&mut frames).await.as_deref(),
        Some(THINKING_FRAME)
    );
    assert_eq!(next_frame(&mut frames).await.as_deref(), Some(ANSWER_FRAME));
    assert_eq!(next_frame(&mut frames).await.as_deref(), Some(DONE_FRAME));
    assert_eq!(next_frame(&mut frames).await, None);
}

#[tokio::test(start_paused = true)]
async fn stream_pings_while_the_model_holds_the_answer() {
    let gate = Arc::new(Notify::new());
    let model = FakeModel::new().waiting(gate.clone());
    let factory = FakeFactory::new(model, FakeSource::new());
    let request = with_principal(post(STREAM_BODY), master_principal());
    let response = send(
        state(
            factory,
            Some(PingSetting::Seconds(1.0)),
            FakeCallerId::from_subject(),
        ),
        request,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut frames = frames(response);
    assert_eq!(
        next_frame(&mut frames).await.as_deref(),
        Some(THINKING_FRAME)
    );
    assert_eq!(next_frame(&mut frames).await.as_deref(), Some(PING_FRAME));
    gate.notify_one();
    assert_eq!(next_frame(&mut frames).await.as_deref(), Some(ANSWER_FRAME));
    assert_eq!(next_frame(&mut frames).await.as_deref(), Some(DONE_FRAME));
    assert_eq!(next_frame(&mut frames).await, None);
}

#[tokio::test(start_paused = true)]
async fn stream_pings_again_after_each_interval() {
    let gate = Arc::new(Notify::new());
    let model = FakeModel::new().waiting(gate.clone());
    let factory = FakeFactory::new(model, FakeSource::new());
    let request = with_principal(post(STREAM_BODY), master_principal());
    let response = send(
        state(
            factory,
            Some(PingSetting::Seconds(2.0)),
            FakeCallerId::from_subject(),
        ),
        request,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut frames = frames(response);
    assert_eq!(
        next_frame(&mut frames).await.as_deref(),
        Some(THINKING_FRAME)
    );
    assert_eq!(next_frame(&mut frames).await.as_deref(), Some(PING_FRAME));
    assert_eq!(next_frame(&mut frames).await.as_deref(), Some(PING_FRAME));
    gate.notify_one();
    assert_eq!(next_frame(&mut frames).await.as_deref(), Some(ANSWER_FRAME));
    assert_eq!(next_frame(&mut frames).await.as_deref(), Some(DONE_FRAME));
}

#[tokio::test(start_paused = true)]
async fn stream_adds_no_ping_without_an_interval() {
    let gate = Arc::new(Notify::new());
    let model = FakeModel::new().waiting(gate.clone());
    let factory = FakeFactory::new(model, FakeSource::new());
    let request = with_principal(post(STREAM_BODY), master_principal());
    let response = send(state(factory, None, FakeCallerId::from_subject()), request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let mut frames = frames(response);
    assert_eq!(
        next_frame(&mut frames).await.as_deref(),
        Some(THINKING_FRAME)
    );
    assert!(
        timeout(Duration::from_secs(60), next_frame(&mut frames))
            .await
            .is_err()
    );
    gate.notify_one();
    assert_eq!(next_frame(&mut frames).await.as_deref(), Some(ANSWER_FRAME));
    assert_eq!(next_frame(&mut frames).await.as_deref(), Some(DONE_FRAME));
}
