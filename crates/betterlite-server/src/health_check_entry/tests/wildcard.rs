#[tokio::test]
async fn wildcard_star_model_returns_without_processing_headers() {
    let harness = Harness::new();
    harness.resolver.resolve_to(resolved("openai/*", "openai"));
    harness.cheapest.set(&["cheap-1", "cheap-2", "cheap-3"]);
    harness.backend.reply(&HEALTHY_HEADERS);
    let outcome = harness
        .run(params(&[("model", "openai/*")]), Some("chat"))
        .await;
    assert_eq!(healthy(outcome), HealthCheckResponse::default());
    let call = harness.backend.take();
    assert_eq!(call.name(), "chat");
    assert_eq!(call.parameters()["model"], Json::Text("cheap-1".to_owned()));
    assert_eq!(
        call.parameters()["fallbacks"],
        Json::List(vec![
            Json::Text("cheap-2".to_owned()),
            Json::Text("cheap-3".to_owned())
        ])
    );
    assert_eq!(
        call.parameters()["max_tokens"],
        Json::Number(Number::from(16_u8))
    );
    assert_eq!(harness.cheapest.request(), Some(("openai".to_owned(), 3)));
}

#[tokio::test]
async fn wildcard_star_model_skips_the_mode_check() {
    let harness = Harness::new();
    harness.resolver.resolve_to(resolved("*", "openai"));
    harness.cheapest.set(&["cheap-1"]);
    let outcome = harness.run(params(&[("model", "*")]), Some("bogus")).await;
    assert_eq!(healthy(outcome), HealthCheckResponse::default());
    let call = harness.backend.take();
    assert_eq!(call.name(), "chat");
    assert_eq!(call.parameters()["model"], Json::Text("cheap-1".to_owned()));
}

#[tokio::test]
async fn wildcard_without_cheap_models_fails_with_the_provider_message() {
    let harness = Harness::new();
    harness.resolver.resolve_to(resolved("openai/*", "openai"));
    let outcome = harness
        .run(params(&[("model", "openai/*")]), Some("chat"))
        .await;
    let failure = failed(outcome);
    let message = "Unable to health check wildcard model for provider openai. Add a model on your config.yaml or contribute here - https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json";
    assert_eq!(failure.error, format!("{message}\nstack trace: {message}"));
    assert!(matches!(failure.exception, HealthEntryError::Probe { .. }));
    assert_eq!(harness.backend.count(), 0);
}

#[tokio::test]
async fn wildcard_searches_the_resolved_provider() {
    let harness = Harness::new();
    harness.resolver.resolve_to(resolved("azure/*", "azure"));
    harness.cheapest.set(&["cheap-1"]);
    let outcome = harness
        .run(params(&[("model", "openai/*")]), Some("chat"))
        .await;
    assert_eq!(healthy(outcome), HealthCheckResponse::default());
    assert_eq!(harness.cheapest.request(), Some(("azure".to_owned(), 3)));
}
