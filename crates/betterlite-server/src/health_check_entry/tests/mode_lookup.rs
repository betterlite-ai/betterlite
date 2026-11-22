#[tokio::test]
async fn mode_takes_the_catalog_mode_of_the_original_model() {
    let harness = Harness::new();
    harness
        .catalog
        .set("text-embedding-3-small", Some("embedding"));
    harness.resolver.resolve_to(resolved("gpt-4o", "openai"));
    let outcome = harness
        .run(
            params(serde_json::json!({"model": "text-embedding-3-small"})),
            None,
        )
        .await;
    assert_eq!(healthy(outcome), HealthCheckResponse::default());
    assert_eq!(harness.backend.take().name(), "embedding");
}

#[tokio::test]
async fn mode_takes_the_catalog_mode_of_the_resolved_model() {
    let harness = Harness::new();
    harness
        .catalog
        .set("text-embedding-3-small", Some("embedding"));
    harness
        .resolver
        .resolve_to(resolved("text-embedding-3-small", "openai"));
    let outcome = harness
        .run(
            params(serde_json::json!({"model": "openai/text-embedding-3-small"})),
            None,
        )
        .await;
    assert_eq!(healthy(outcome), HealthCheckResponse::default());
    assert_eq!(harness.backend.take().name(), "embedding");
}

#[tokio::test]
async fn mode_treats_a_catalog_entry_without_a_mode_as_unset() {
    let harness = Harness::new();
    harness.catalog.set("fireworks-ai-default", None);
    let outcome = harness
        .run(
            params(serde_json::json!({"model": "fireworks-ai-default"})),
            None,
        )
        .await;
    assert_eq!(healthy(outcome), HealthCheckResponse::default());
    assert_eq!(harness.backend.take().name(), "chat");
}

#[tokio::test]
async fn mode_ignores_the_catalog_for_an_explicit_mode() {
    let harness = Harness::new();
    harness
        .catalog
        .set("text-embedding-3-small", Some("embedding"));
    let outcome = harness
        .run(
            params(serde_json::json!({"model": "text-embedding-3-small"})),
            Some("chat"),
        )
        .await;
    assert_eq!(healthy(outcome), HealthCheckResponse::default());
    assert_eq!(harness.backend.take().name(), "chat");
}

#[tokio::test]
async fn mode_catalogs_neither_model_for_an_empty_mode() {
    let harness = Harness::new();
    harness
        .catalog
        .set("text-embedding-3-small", Some("embedding"));
    harness.catalog.set("resolved-model", Some("embedding"));
    let outcome = harness
        .run(
            params(serde_json::json!({"model": "text-embedding-3-small"})),
            Some(""),
        )
        .await;
    assert_eq!(healthy(outcome), HealthCheckResponse::default());
    assert_eq!(harness.backend.take().name(), "chat");
}

#[tokio::test]
async fn mode_missing_model_fails_with_model_not_set() {
    let harness = Harness::new();
    let outcome = harness
        .run(params(serde_json::json!({})), Some("chat"))
        .await;
    let failure = failed(outcome);
    assert_eq!(failure.error, "model not set\nstack trace: model not set");
    assert!(matches!(failure.exception, HealthEntryError::ModelNotSet));
    assert_eq!(harness.backend.count(), 0);
}

#[tokio::test]
async fn mode_missing_model_without_a_mode_uses_the_missing_mode_text() {
    let harness = Harness::new();
    let outcome = harness.run(params(serde_json::json!({})), None).await;
    let failure = failed(outcome);
    assert_eq!(
        failure.error,
        "error:model not set. Missing `mode`. Set the `mode` for the model - https://docs.litellm.ai/docs/proxy/health#embedding-models  \nstacktrace: model not set"
    );
    assert!(failure.raw_request.is_none());
    assert!(matches!(failure.exception, HealthEntryError::ModelNotSet));
}

// the one place ModelCatalog's CatalogModes impl runs: every catalog mode string is Mode's serde name
#[rstest]
#[case::audio_speech("audio_speech")]
#[case::audio_transcription("audio_transcription")]
#[case::chat("chat")]
#[case::completion("completion")]
#[case::embedding("embedding")]
#[case::evaluation("evaluation")]
#[case::guardrail("guardrail")]
#[case::image_edit("image_edit")]
#[case::image_generation("image_generation")]
#[case::moderation("moderation")]
#[case::ocr("ocr")]
#[case::realtime("realtime")]
#[case::rerank("rerank")]
#[case::responses("responses")]
#[case::search("search")]
#[case::vector_store("vector_store")]
#[case::video_generation("video_generation")]
fn mode_reads_each_catalog_mode_name(#[case] name: &str) {
    let document = format!(r#"{{"probe-model": {{"mode": "{name}"}}}}"#);
    let catalog = ModelCatalog::parse(document.as_bytes(), UnknownFields::Reject)
        .expect("the probe catalog parses");
    assert_eq!(
        catalog.catalog_mode("probe-model"),
        CatalogMode::Mode(name.to_owned())
    );
}

#[test]
fn mode_treats_a_missing_or_unlisted_catalog_entry_as_unset() {
    let catalog = ModelCatalog::parse(br#"{"probe-model": {}}"#, UnknownFields::Reject)
        .expect("the probe catalog parses");
    assert_eq!(catalog.catalog_mode("probe-model"), CatalogMode::NoMode);
    assert_eq!(
        catalog.catalog_mode("unlisted-model"),
        CatalogMode::NotListed
    );
}

#[tokio::test]
async fn mode_resolution_receives_the_original_model_string() {
    let harness = Harness::new();
    harness.resolver.failing();
    let outcome = harness
        .run(
            params(serde_json::json!({"model": "openai/unknown"})),
            Some("chat"),
        )
        .await;
    let failure = failed(outcome);
    let message = "LLM Provider NOT provided. Pass in the LLM provider you are trying to call. You passed model=openai/unknown";
    assert_eq!(
        failure.error,
        format!("{message}\nstack trace: {message}\n{message}")
    );
    assert!(matches!(
        failure.exception,
        HealthEntryError::Resolution { .. }
    ));
    assert_eq!(
        harness.resolver.request().map(|request| request.model),
        Some("openai/unknown".to_owned())
    );
}
