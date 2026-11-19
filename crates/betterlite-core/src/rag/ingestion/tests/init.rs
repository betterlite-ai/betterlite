use betterlite_config::rag::document::ingest::RAGIngestOCROptions;

use super::super::chunk::ChunkPlan;
use super::*;

#[test]
fn init_generates_a_prefixed_uuid_id() {
    let state = Harness::default().state(IngestionOptions::default());
    let suffix = state.ingest_id.strip_prefix("ingest_").expect("the prefix");
    assert_eq!(suffix.len(), 36);
    uuid::Uuid::parse_str(suffix).unwrap();
}

#[test]
fn init_stores_the_ingest_options() {
    let parsed = serde_json::from_str(
        r#"{
            "name": "docs",
            "ocr": {"model": "mistral/mistral-ocr-latest"},
            "embedding": {"model": "text-embedding-3-small"},
            "vector_store": {"custom_llm_provider": "openai", "vector_store_id": "vs_1"}
        }"#,
    )
    .expect("the options parse");
    let state = Harness::default().state(parsed);
    assert_eq!(state.ingest_name.as_deref(), Some("docs"));
    assert_eq!(
        state.ocr_config,
        Some(RAGIngestOCROptions {
            model: Some("mistral/mistral-ocr-latest".to_owned())
        })
    );
    assert_eq!(
        state
            .embedding_config
            .as_ref()
            .and_then(|config| config.model.as_deref()),
        Some("text-embedding-3-small")
    );
    assert_eq!(
        text_of(&state.vector_store_config, "custom_llm_provider"),
        "openai"
    );
    assert_eq!(
        text_of(&state.vector_store_config, "vector_store_id"),
        "vs_1"
    );
}

#[test]
fn init_defaults_the_chunking_strategy_to_auto() {
    let state = Harness::default().state(IngestionOptions::default());
    assert_eq!(state.chunking_strategy.get(), r#"{"type":"auto"}"#);
    assert!(state.vector_store_config.is_empty());
    assert!(state.ocr_config.is_none());
}

#[test]
fn init_defaults_a_null_chunking_strategy() {
    let parsed = serde_json::from_str(r#"{"chunking_strategy": null}"#).expect("the options parse");
    let state = Harness::default().state(parsed);
    assert_eq!(state.chunking_strategy.get(), r#"{"type":"auto"}"#);
}

#[test]
fn init_keeps_an_empty_chunking_strategy() {
    let parsed = serde_json::from_str(r#"{"chunking_strategy": {}}"#).expect("the options parse");
    let state = Harness::default().state(parsed);
    assert_eq!(state.chunking_strategy.get(), "{}");
}

#[test]
fn init_keeps_a_configured_chunking_strategy() {
    let parsed = serde_json::from_str(r#"{"chunking_strategy": {"chunk_size": 5}}"#)
        .expect("the options parse");
    let state = Harness::default().state(parsed);
    let plan = ChunkPlan::from_strategy(&state.chunking_strategy).expect("the plan");
    assert_eq!(plan.size, 5);
    assert_eq!(plan.overlap, 200);
}

#[test]
fn init_leaves_supports_existing_file_id_false() {
    let supported = IngestionState::SUPPORTS_EXISTING_FILE_ID;
    assert!(!supported);
}

#[test]
fn init_rejects_an_unknown_ingest_option() {
    let parsed = serde_json::from_str::<IngestionOptions>(r#"{"unexpected_option": "docs"}"#);
    parsed.expect_err("an unknown option is rejected");
}
