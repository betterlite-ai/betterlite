async fn failing_after_content(class: ErrorClass, provider: Option<&str>) -> ChatStreamError {
    let mut config = config(CallMode::Async);
    config.provider = provider.map(str::to_owned);
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::scripted(vec![
        Ok(content_chunk("He")),
        Err(source_error(SourceFailure::Other, class, "upstream died")),
    ]);
    let mut stream = start(&world, &mut correlation, config, source);
    assert_eq!(content_of(&only_chunk(stream.next().await)), "He");
    stream.next().await.unwrap_err()
}

#[tokio::test]
async fn fallback_bad_request_raises_directly() {
    let error = failing_after_content(bad_request_class("bad input"), None).await;
    assert!(matches!(error.class, ErrorClass::BadRequest(_)));
    assert_eq!(error.class.metadata().status_code, 400);
}

#[tokio::test]
async fn fallback_rate_limit_becomes_a_fallback() {
    let error = failing_after_content(rate_limited_class("slow down"), None).await;
    let details = fallback_of(&error);
    assert_eq!(details.original_status.0, 429);
}

#[tokio::test]
async fn fallback_timeout_status_raises_directly() {
    let error = failing_after_content(timeout_class("timed out"), None).await;
    assert!(matches!(error.class, ErrorClass::Timeout(_)));
    assert_eq!(error.class.metadata().status_code, 408);
}

#[tokio::test]
async fn fallback_bad_gateway_keeps_generated_content() {
    let error = failing_after_content(bad_gateway_class("upstream died"), None).await;
    let details = fallback_of(&error);
    assert_eq!(details.generated_content, "He");
    assert!(!details.is_pre_first_chunk);
    assert_eq!(details.original_status.0, 502);
    assert_eq!(details.context.model.as_deref(), Some(MODEL));
    assert_eq!(details.context.provider.as_deref(), Some("anthropic"));
}

#[tokio::test]
async fn fallback_empty_provider_uses_anthropic() {
    let error = failing_after_content(bad_gateway_class("upstream died"), Some("")).await;
    assert_eq!(
        fallback_of(&error).context.provider.as_deref(),
        Some("anthropic")
    );
}

#[tokio::test]
async fn fallback_keeps_the_configured_provider() {
    let error = failing_after_content(bad_gateway_class("upstream died"), Some("openai")).await;
    assert_eq!(
        fallback_of(&error).context.provider.as_deref(),
        Some("openai")
    );
}

#[tokio::test]
async fn fallback_without_content_is_pre_first_chunk() {
    let world = World::new();
    let mut correlation = CorrelationContext::default();
    let source = FakeSource::scripted(vec![Err(source_error(
        SourceFailure::Other,
        bad_gateway_class("upstream died"),
        "upstream died",
    ))]);
    let mut stream = start(&world, &mut correlation, config(CallMode::Async), source);

    let error = stream.next().await.unwrap_err();
    let details = fallback_of(&error);
    assert!(details.is_pre_first_chunk);
    assert!(details.generated_content.is_empty());
}
