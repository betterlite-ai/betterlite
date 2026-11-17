#[tokio::test]
async fn headers_mark_the_stream_as_server_sent_events() {
    let model = FakeModel::new().replying(answer_reply("Hi"));
    let factory = FakeFactory::new(model, FakeSource::new());
    let request = with_principal(post(STREAM_BODY), master_principal());
    let response = send(state(factory, None, FakeCallerId::from_subject()), request).await;
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(headers.len(), 3);
    assert_eq!(
        headers.get(CONTENT_TYPE),
        Some(&HeaderValue::from_static(
            "text/event-stream; charset=utf-8"
        ))
    );
    assert_eq!(
        headers.get(CACHE_CONTROL),
        Some(&HeaderValue::from_static("no-cache"))
    );
    assert_eq!(
        headers.get("x-accel-buffering"),
        Some(&HeaderValue::from_static("no"))
    );
}
