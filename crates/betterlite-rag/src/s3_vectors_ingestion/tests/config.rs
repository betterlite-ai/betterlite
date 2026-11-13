use betterlite_config::rag::document::ingest::{
    RAGIngestEmbeddingOptions, S3VectorsVectorStoreOptions,
};
use rstest::rstest;

use crate::s3_vectors_ingestion::{
    DEFAULT_NON_FILTERABLE_KEYS, S3_VECTORS_STORE_ID_ERROR, ingest_embedding_options, ingest_target,
};

fn options(text: &str) -> S3VectorsVectorStoreOptions {
    serde_json::from_str(text).expect("the fixture vector store options are valid")
}

fn embedding_options(model: &str) -> RAGIngestEmbeddingOptions {
    RAGIngestEmbeddingOptions {
        model: Some(model.to_owned()),
    }
}

#[test]
fn ingest_target_split_returns_both_parts() {
    let target = ingest_target(&options(r#"{"vector_store_id": "bucket:index"}"#))
        .expect("a vector store target");
    assert_eq!(target, ("bucket".to_owned(), Some("index".to_owned())));
}

#[test]
fn ingest_target_split_prefers_the_explicit_parts() {
    let target = ingest_target(&options(
        r#"{"vector_store_id": "id-bucket:id-index", "vector_bucket_name": "explicit-bucket", "index_name": "explicit-index"}"#,
    ))
    .expect("a vector store target");
    assert_eq!(
        target,
        (
            "explicit-bucket".to_owned(),
            Some("explicit-index".to_owned())
        )
    );
}

#[test]
fn ingest_target_fallback_uses_the_bucket_name() {
    let target = ingest_target(&options(
        r#"{"vector_store_id": "index", "vector_bucket_name": "bucket"}"#,
    ))
    .expect("a vector store target");
    assert_eq!(target, ("bucket".to_owned(), Some("index".to_owned())));
}

#[test]
fn ingest_target_missing_bucket_errors() {
    let error = ingest_target(&options(r#"{"vector_store_id": "index"}"#))
        .expect_err("a store id without a bucket is rejected");
    assert_eq!(error.to_string(), S3_VECTORS_STORE_ID_ERROR);
}

#[test]
fn ingest_target_uses_the_configured_bucket_without_a_store_id() {
    let target = ingest_target(&options(r#"{"vector_bucket_name": "bucket"}"#)).expect("a target");
    assert_eq!(target, ("bucket".to_owned(), None));
}

#[test]
fn ingest_target_uses_the_configured_index_name() {
    let target = ingest_target(&options(
        r#"{"vector_bucket_name": "bucket", "index_name": "index"}"#,
    ))
    .expect("a target");
    assert_eq!(target, ("bucket".to_owned(), Some("index".to_owned())));
}

#[test]
fn ingest_target_treats_an_empty_bucket_name_as_absent() {
    let error = ingest_target(&options(
        r#"{"vector_store_id": "index", "vector_bucket_name": ""}"#,
    ))
    .expect_err("an empty fallback bucket is rejected");
    assert_eq!(error.to_string(), S3_VECTORS_STORE_ID_ERROR);
}

#[rstest]
#[case::empty_bucket(":index")]
#[case::empty_index("bucket:")]
fn ingest_target_rejects_empty_id_parts(#[case] store_id: &str) {
    let text = format!(r#"{{"vector_store_id": "{store_id}", "vector_bucket_name": "bucket"}}"#);
    let error = ingest_target(&options(&text)).expect_err("an empty part is rejected");
    assert_eq!(error.to_string(), S3_VECTORS_STORE_ID_ERROR);
}

#[test]
fn ingest_embedding_override_uses_the_litellm_model() {
    let config =
        options(r#"{"litellm_embedding_model": "store-model", "embedding_model": "other-model"}"#);
    let resolved = ingest_embedding_options(&config, Some(embedding_options("request-model")))
        .expect("the store model overrides the request");
    assert_eq!(resolved.model.as_deref(), Some("store-model"));
}

#[test]
fn ingest_embedding_override_falls_back_to_the_embedding_model() {
    let config = options(r#"{"embedding_model": "other-model"}"#);
    let resolved = ingest_embedding_options(&config, Some(embedding_options("request-model")))
        .expect("the store model overrides the request");
    assert_eq!(resolved.model.as_deref(), Some("other-model"));
}

#[test]
fn ingest_embedding_override_skips_an_empty_litellm_model() {
    let config = options(r#"{"litellm_embedding_model": "", "embedding_model": "other-model"}"#);
    let resolved = ingest_embedding_options(&config, Some(embedding_options("request-model")))
        .expect("the fallback model overrides the request");
    assert_eq!(resolved.model.as_deref(), Some("other-model"));
}

#[test]
fn ingest_embedding_override_leaves_the_input_unchanged() {
    let config = options("{}");
    let resolved = ingest_embedding_options(&config, Some(embedding_options("request-model")));
    assert_eq!(resolved, Some(embedding_options("request-model")));
}

#[test]
fn ingest_embedding_override_keeps_absent_options() {
    let config = options("{}");
    assert_eq!(ingest_embedding_options(&config, None), None);
}

#[test]
fn default_non_filterable_keys_are_source_text() {
    assert_eq!(DEFAULT_NON_FILTERABLE_KEYS, ["source_text"]);
}
