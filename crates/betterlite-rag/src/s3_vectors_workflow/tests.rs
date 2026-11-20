#![cfg(test)]

mod fixtures;
mod support;

use std::collections::BTreeMap;

use betterlite_config::rag::document::ingest::{RAGIngestEmbeddingOptions, SslVerify};
use betterlite_core::rag::ingestion::store::{RagStore, StoreRequest};
use secrecy::ExposeSecret as _;

use self::fixtures::{
    accepted, bucket_call, entry, environment, index_call, not_found, options, rag_workflow,
    s3_settings, s3_settings_in,
};
use self::support::{
    Call, Failure, FakeApi, FakeEmbedder, FixedClock, PutVectorsBody, RecordingTransport,
    StaticResolver, UNIQUE_ID,
};
use super::dimension::{EmbedError, detect_dimension};
use super::settings::{DEFAULT_DIMENSION, S3VectorsSettings, SettingsError};
use super::workflow::signed::SignedS3Vectors;
use super::workflow::{NO_CONTENT_MESSAGE, S3VectorsWorkflow, WorkflowError};
use crate::s3_vectors_ingestion::S3_VECTORS_STORE_ID_ERROR;

#[test]
fn settings_take_bucket_and_index_from_the_target() {
    let from_id = s3_settings(r#"{"vector_store_id": "my-bucket:my-index"}"#);
    assert_eq!(from_id.vector_bucket_name, "my-bucket");
    assert_eq!(from_id.index_name.as_deref(), Some("my-index"));

    let from_bucket = s3_settings(r#"{"vector_bucket_name": "bucket-only"}"#);
    assert_eq!(from_bucket.vector_bucket_name, "bucket-only");
    assert_eq!(from_bucket.index_name, None);

    let explicit_index = s3_settings(r#"{"vector_bucket_name": "b", "index_name": "idx"}"#);
    assert_eq!(explicit_index.index_name.as_deref(), Some("idx"));
}

#[test]
fn settings_apply_the_litellm_defaults() {
    let settings = s3_settings(r#"{"vector_bucket_name": "my-bucket"}"#);
    assert_eq!(settings.distance_metric, "cosine");
    assert_eq!(
        settings.non_filterable_metadata_keys,
        vec!["source_text".to_owned()]
    );
    assert_eq!(settings.dimension, None);
    assert_eq!(settings.default_dimension, DEFAULT_DIMENSION);
    assert_eq!(settings.aws_region_name, "us-west-2");
    assert_eq!(settings.embedding_model, None);
    assert_eq!(settings.ssl_verify, None);
    assert!(settings.credentials.access_key_id.is_none());
}

#[test]
fn settings_read_the_environment_defaults() {
    let settings = s3_settings_in(
        r#"{"vector_bucket_name": "my-bucket"}"#,
        &[
            ("S3_VECTORS_DEFAULT_DISTANCE_METRIC", "euclidean"),
            ("S3_VECTORS_DEFAULT_DIMENSION", "256"),
            ("AWS_REGION_NAME", "eu-west-1"),
        ],
    );
    assert_eq!(settings.distance_metric, "euclidean");
    assert_eq!(settings.default_dimension, 256);
    assert_eq!(settings.aws_region_name, "eu-west-1");
}

#[test]
fn settings_prefer_the_configured_distance_metric() {
    let configured = s3_settings(r#"{"vector_bucket_name": "b", "distance_metric": "euclidean"}"#);
    assert_eq!(configured.distance_metric, "euclidean");

    let configured_wins = s3_settings_in(
        r#"{"vector_bucket_name": "b", "distance_metric": "cosine"}"#,
        &[("S3_VECTORS_DEFAULT_DISTANCE_METRIC", "euclidean")],
    );
    assert_eq!(configured_wins.distance_metric, "cosine");
}

#[test]
fn settings_treat_an_empty_environment_value_as_unset() {
    let settings = s3_settings_in(
        r#"{"vector_bucket_name": "my-bucket"}"#,
        &[
            ("S3_VECTORS_DEFAULT_DISTANCE_METRIC", ""),
            ("S3_VECTORS_DEFAULT_DIMENSION", ""),
        ],
    );
    assert_eq!(settings.distance_metric, "cosine");
    assert_eq!(settings.default_dimension, DEFAULT_DIMENSION);
}

#[test]
fn settings_read_the_region_explicit_then_aws_region() {
    let explicit = s3_settings_in(
        r#"{"vector_bucket_name": "b", "aws_region_name": "ap-south-1"}"#,
        &[("AWS_REGION", "eu-west-1")],
    );
    assert_eq!(explicit.aws_region_name, "ap-south-1");

    let standard = s3_settings_in(
        r#"{"vector_bucket_name": "b"}"#,
        &[
            ("AWS_REGION_NAME", "eu-west-1"),
            ("AWS_REGION", "sa-east-1"),
        ],
    );
    assert_eq!(standard.aws_region_name, "sa-east-1");

    let empty_explicit = s3_settings_in(
        r#"{"vector_bucket_name": "b", "aws_region_name": ""}"#,
        &[("AWS_REGION_NAME", "eu-west-1")],
    );
    assert_eq!(empty_explicit.aws_region_name, "eu-west-1");
}

#[test]
fn settings_set_the_dimension_only_when_the_key_is_present() {
    let configured = s3_settings(r#"{"vector_bucket_name": "b", "dimension": 512}"#);
    assert_eq!(configured.dimension, Some(512));

    let detected = s3_settings_in(
        r#"{"vector_bucket_name": "b"}"#,
        &[("S3_VECTORS_DEFAULT_DIMENSION", "256")],
    );
    assert_eq!(detected.dimension, None);
    assert_eq!(detected.default_dimension, 256);
}

#[test]
fn settings_take_the_embedding_model_from_the_store_or_the_request() {
    let request = Some(RAGIngestEmbeddingOptions {
        model: Some("request/model".to_owned()),
    });
    let from_litellm_model = S3VectorsSettings::from_vector_store_config(
        &options(r#"{"vector_bucket_name": "b", "litellm_embedding_model": "store/model"}"#),
        request.clone(),
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        from_litellm_model.embedding_model.as_deref(),
        Some("store/model")
    );

    let from_embedding_model = S3VectorsSettings::from_vector_store_config(
        &options(r#"{"vector_bucket_name": "b", "embedding_model": "store/model"}"#),
        request.clone(),
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        from_embedding_model.embedding_model.as_deref(),
        Some("store/model")
    );

    let passthrough = S3VectorsSettings::from_vector_store_config(
        &options(r#"{"vector_bucket_name": "b"}"#),
        request,
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(
        passthrough.embedding_model.as_deref(),
        Some("request/model")
    );

    let absent = s3_settings(r#"{"vector_bucket_name": "b", "litellm_embedding_model": ""}"#);
    assert_eq!(absent.embedding_model, None);
}

#[test]
fn settings_keep_the_metadata_keys_and_ssl_verify() {
    let settings = s3_settings(
        r#"{"vector_bucket_name": "b", "non_filterable_metadata_keys": ["a", "b"], "ssl_verify": false}"#,
    );
    assert_eq!(
        settings.non_filterable_metadata_keys,
        vec!["a".to_owned(), "b".to_owned()]
    );
    assert_eq!(settings.ssl_verify, Some(SslVerify::Bool(false)));

    let empty = s3_settings(r#"{"vector_bucket_name": "b", "non_filterable_metadata_keys": []}"#);
    assert!(empty.non_filterable_metadata_keys.is_empty());

    let bundle = s3_settings(r#"{"vector_bucket_name": "b", "ssl_verify": "/certs/ca.pem"}"#);
    assert_eq!(
        bundle.ssl_verify,
        Some(SslVerify::CaBundle("/certs/ca.pem".to_owned()))
    );
}

#[test]
fn settings_bad_target_is_a_typed_error_with_the_litellm_text() {
    let error = S3VectorsSettings::from_vector_store_config(&options("{}"), None, &BTreeMap::new())
        .unwrap_err();
    assert!(matches!(error, SettingsError::Target(_)));
    assert_eq!(error.to_string(), S3_VECTORS_STORE_ID_ERROR);
}

#[test]
fn settings_bad_environment_values_are_typed_errors() {
    let dimension = S3VectorsSettings::from_vector_store_config(
        &options(r#"{"vector_bucket_name": "b"}"#),
        None,
        &environment(&[("S3_VECTORS_DEFAULT_DIMENSION", "many")]),
    );
    assert!(matches!(dimension, Err(SettingsError::Environment(_))));

    let region = S3VectorsSettings::from_vector_store_config(
        &options(r#"{"vector_bucket_name": "b", "aws_region_name": "EU_WEST"}"#),
        None,
        &BTreeMap::new(),
    );
    assert!(matches!(region, Err(SettingsError::Region(_))));
}

#[test]
fn settings_keep_the_credentials_out_of_errors_and_debug() {
    let secret = r#"{"aws_secret_access_key": "super-secret-value"}"#;
    let error =
        S3VectorsSettings::from_vector_store_config(&options(secret), None, &BTreeMap::new())
            .unwrap_err();
    let text = format!("{error} {error:?}");
    assert!(!text.contains("super-secret-value"));

    let settings = s3_settings(
        r#"{"vector_bucket_name": "b", "aws_access_key_id": "AKIDEXAMPLE", "aws_secret_access_key": "vectors-secret", "aws_session_token": "session-token", "aws_session_name": "session-name", "aws_profile_name": "vectors-profile", "aws_role_name": "vectors-role", "aws_web_identity_token": "web-identity-token", "aws_sts_endpoint": "https://sts.example", "aws_external_id": "external"}"#,
    );
    assert_credentials(&settings);
    assert!(!format!("{settings:?}").contains("AKIDEXAMPLE"));
}

fn assert_credentials(settings: &S3VectorsSettings) {
    let credentials = &settings.credentials;
    assert_eq!(
        credentials.access_key_id.as_ref().unwrap().expose_secret(),
        "AKIDEXAMPLE"
    );
    assert_eq!(
        credentials
            .secret_access_key
            .as_ref()
            .unwrap()
            .expose_secret(),
        "vectors-secret"
    );
    assert_eq!(
        credentials.session_token.as_ref().unwrap().expose_secret(),
        "session-token"
    );
    assert_eq!(credentials.session_name.as_deref(), Some("session-name"));
    assert_eq!(credentials.profile_name.as_deref(), Some("vectors-profile"));
    assert_eq!(credentials.role_name.as_deref(), Some("vectors-role"));
    assert_eq!(
        credentials
            .web_identity_token
            .as_ref()
            .unwrap()
            .expose_secret(),
        "web-identity-token"
    );
    assert_eq!(
        credentials.sts_endpoint.as_deref(),
        Some("https://sts.example")
    );
    assert_eq!(credentials.external_id.as_deref(), Some("external"));
}

#[tokio::test]
async fn dimension_without_a_model_sends_nothing() {
    let embedder = FakeEmbedder::new(vec![vec![1.0_f64, 2.0_f64]], false);
    let dimension = detect_dimension(&embedder, None, DEFAULT_DIMENSION).await;
    assert_eq!(dimension, DEFAULT_DIMENSION);
    assert!(embedder.calls().is_empty());
}

#[tokio::test]
async fn dimension_probes_with_the_single_test_input() {
    let embedder = FakeEmbedder::new(vec![vec![1.0_f64, 2.0_f64, 3.0_f64], vec![4.0_f64]], false);
    let dimension = detect_dimension(&embedder, Some("vendor/model"), 512).await;
    assert_eq!(dimension, 3);
    assert_eq!(
        embedder.calls(),
        vec![("vendor/model".to_owned(), vec!["test".to_owned()])]
    );
}

#[tokio::test]
async fn dimension_empty_answer_returns_the_default() {
    let empty = FakeEmbedder::new(Vec::new(), false);
    assert_eq!(detect_dimension(&empty, Some("m"), 512).await, 512);

    let zero_length = FakeEmbedder::new(vec![Vec::new()], false);
    assert_eq!(detect_dimension(&zero_length, Some("m"), 512).await, 0);
}

#[tokio::test]
async fn dimension_embed_error_returns_the_default() {
    let embedder = FakeEmbedder::new(vec![vec![1.0_f64]], true);
    assert_eq!(detect_dimension(&embedder, Some("m"), 512).await, 512);
}

#[tokio::test]
async fn flow_creates_the_bucket_then_the_generated_index() {
    let api = FakeApi::new();
    let workflow = S3VectorsWorkflow::new(
        s3_settings(r#"{"vector_bucket_name": "flow-bucket"}"#),
        api.clone(),
        FakeEmbedder::new(Vec::new(), false),
        UNIQUE_ID.to_owned(),
    );
    let resolved = workflow.ensure_config_initialized().await.unwrap();
    assert_eq!(resolved.bucket_name, "flow-bucket");
    assert_eq!(resolved.index_name, "litellm-index-abcd1234");
    assert_eq!(
        api.calls(),
        vec![
            bucket_call("flow-bucket"),
            index_call("flow-bucket", "litellm-index-abcd1234", DEFAULT_DIMENSION),
        ]
    );
}

#[tokio::test]
async fn flow_probes_only_when_the_dimension_is_unset() {
    let api = FakeApi::new();
    let embedder = FakeEmbedder::new(vec![vec![1.0_f64, 2.0_f64, 3.0_f64]], false);
    let workflow = S3VectorsWorkflow::new(
        s3_settings(
            r#"{"vector_bucket_name": "b", "index_name": "idx", "dimension": 512, "litellm_embedding_model": "vendor/model"}"#,
        ),
        api.clone(),
        embedder.clone(),
        UNIQUE_ID.to_owned(),
    );
    workflow.ensure_config_initialized().await.unwrap();
    assert!(embedder.calls().is_empty());
    assert_eq!(
        api.calls(),
        vec![bucket_call("b"), index_call("b", "idx", 512)]
    );
}

#[tokio::test]
async fn flow_probes_the_model_and_uses_the_first_vector_length() {
    let api = FakeApi::new();
    let embedder = FakeEmbedder::new(vec![vec![1.0_f64, 2.0_f64, 3.0_f64, 4.0_f64]], false);
    let workflow = S3VectorsWorkflow::new(
        s3_settings(r#"{"vector_bucket_name": "b", "litellm_embedding_model": "vendor/model"}"#),
        api.clone(),
        embedder.clone(),
        UNIQUE_ID.to_owned(),
    );
    workflow.ensure_config_initialized().await.unwrap();
    assert_eq!(
        embedder.calls(),
        vec![("vendor/model".to_owned(), vec!["test".to_owned()])]
    );
    assert_eq!(
        api.calls(),
        vec![
            bucket_call("b"),
            index_call("b", "litellm-index-abcd1234", 4),
        ]
    );
}

#[tokio::test]
async fn flow_runs_once_per_workflow() {
    let api = FakeApi::new();
    let workflow = S3VectorsWorkflow::new(
        s3_settings(r#"{"vector_bucket_name": "b"}"#),
        api.clone(),
        FakeEmbedder::new(Vec::new(), false),
        UNIQUE_ID.to_owned(),
    );
    workflow.ensure_config_initialized().await.unwrap();
    workflow.ensure_config_initialized().await.unwrap();
    assert_eq!(api.calls().len(), 2);
}

#[tokio::test]
async fn flow_keeps_the_index_name_and_dimension_after_a_failure() {
    let api = FakeApi::failing(vec![Failure::Index]);
    let embedder = FakeEmbedder::new(vec![vec![1.0_f64, 2.0_f64]], false);
    let workflow = S3VectorsWorkflow::new(
        s3_settings(r#"{"vector_bucket_name": "b", "litellm_embedding_model": "vendor/model"}"#),
        api.clone(),
        embedder.clone(),
        UNIQUE_ID.to_owned(),
    );
    let error = workflow.ensure_config_initialized().await.unwrap_err();
    assert_eq!(error.to_string(), "the scripted S3 Vectors API failed");

    let resolved = workflow.ensure_config_initialized().await.unwrap();
    assert_eq!(resolved.index_name, "litellm-index-abcd1234");
    assert_eq!(embedder.calls().len(), 1);
    assert_eq!(
        api.calls(),
        vec![
            bucket_call("b"),
            index_call("b", "litellm-index-abcd1234", 2),
            bucket_call("b"),
            index_call("b", "litellm-index-abcd1234", 2),
        ]
    );
}

#[tokio::test]
async fn flow_retries_the_bucket_without_a_second_probe() {
    let api = FakeApi::failing(vec![Failure::Bucket]);
    let embedder = FakeEmbedder::new(vec![vec![0.5_f64, 0.25_f64]], false);
    let workflow = S3VectorsWorkflow::new(
        s3_settings(r#"{"vector_bucket_name": "b", "litellm_embedding_model": "vendor/model"}"#),
        api.clone(),
        embedder.clone(),
        UNIQUE_ID.to_owned(),
    );
    let error = workflow.ensure_config_initialized().await.unwrap_err();
    assert_eq!(error.to_string(), "the scripted S3 Vectors API failed");
    assert_eq!(embedder.calls().len(), 1);

    let resolved = workflow.ensure_config_initialized().await.unwrap();
    assert_eq!(resolved.index_name, "litellm-index-abcd1234");
    assert_eq!(embedder.calls().len(), 1);
    assert_eq!(
        api.calls(),
        vec![
            bucket_call("b"),
            bucket_call("b"),
            index_call("b", "litellm-index-abcd1234", 2),
        ]
    );
}

#[tokio::test]
async fn embed_returns_none_for_no_chunks() {
    let api = FakeApi::new();
    let embedder = FakeEmbedder::new(vec![vec![1.0_f64]], false);
    let workflow = S3VectorsWorkflow::new(
        s3_settings(r#"{"vector_bucket_name": "b"}"#),
        api,
        embedder.clone(),
        UNIQUE_ID.to_owned(),
    );
    assert_eq!(workflow.embed(&[]).await.unwrap(), None);
    assert!(embedder.calls().is_empty());
}

#[tokio::test]
async fn embed_uses_the_configured_model_in_order() {
    let embedder = FakeEmbedder::new(vec![vec![0.5_f64], vec![0.25_f64]], false);
    let workflow = S3VectorsWorkflow::new(
        s3_settings(r#"{"vector_bucket_name": "b", "litellm_embedding_model": "vendor/model"}"#),
        FakeApi::new(),
        embedder.clone(),
        UNIQUE_ID.to_owned(),
    );
    let chunks = vec!["one".to_owned(), "two".to_owned()];
    let vectors = workflow.embed(&chunks).await.unwrap();
    assert_eq!(vectors, Some(vec![vec![0.5_f64], vec![0.25_f64]]));
    assert_eq!(
        embedder.calls(),
        vec![(
            "vendor/model".to_owned(),
            vec!["one".to_owned(), "two".to_owned()]
        )]
    );
}

#[tokio::test]
async fn embed_falls_back_to_the_default_model_and_stores_it() {
    let embedder = FakeEmbedder::new(vec![vec![0.5_f64]], false);
    let workflow = S3VectorsWorkflow::new(
        s3_settings(r#"{"vector_bucket_name": "b"}"#),
        FakeApi::new(),
        embedder.clone(),
        UNIQUE_ID.to_owned(),
    );
    assert_eq!(
        workflow.embed(&["one".to_owned()]).await.unwrap(),
        Some(vec![vec![0.5_f64]])
    );
    assert_eq!(
        workflow.embed(&["two".to_owned()]).await.unwrap(),
        Some(vec![vec![0.5_f64]])
    );
    assert_eq!(
        embedder.calls(),
        vec![
            ("text-embedding-3-small".to_owned(), vec!["one".to_owned()]),
            ("text-embedding-3-small".to_owned(), vec!["two".to_owned()]),
        ]
    );

    let chunks = vec!["three".to_owned()];
    let embeddings = vec![vec![0.5_f64]];
    workflow
        .store_vectors(Some("a.txt"), &chunks, Some(embeddings.as_slice()))
        .await
        .unwrap();
    let calls = embedder.calls();
    assert_eq!(calls[2].0, "text-embedding-3-small");
    assert_eq!(calls[2].1, vec!["test".to_owned()]);
}

#[tokio::test]
async fn embed_propagates_the_embedder_error() {
    let embedder = FakeEmbedder::new(Vec::new(), true);
    let workflow = S3VectorsWorkflow::new(
        s3_settings(r#"{"vector_bucket_name": "b"}"#),
        FakeApi::new(),
        embedder,
        UNIQUE_ID.to_owned(),
    );
    let error = workflow.embed(&["one".to_owned()]).await.unwrap_err();
    assert!(matches!(
        error,
        WorkflowError::Embed(EmbedError::Failed { .. })
    ));
}

#[tokio::test]
async fn store_builds_an_entry_per_pair_with_the_filename() {
    let api = FakeApi::new();
    let workflow = rag_workflow(&api);
    let chunks = vec!["first".to_owned(), "second".to_owned()];
    let embeddings = vec![vec![0.5_f64, 0.25_f64], vec![0.125_f64, 0.0625_f64]];
    let (id, name) = workflow
        .store_vectors(Some("notes.txt"), &chunks, Some(embeddings.as_slice()))
        .await
        .unwrap();
    assert_eq!(id.as_deref(), Some("rag:idx"));
    assert_eq!(name.as_deref(), Some("notes.txt"));
    assert_eq!(
        api.calls(),
        vec![
            bucket_call("rag"),
            index_call("rag", "idx", 4),
            Call::Vectors {
                bucket_name: "rag".to_owned(),
                index_name: "idx".to_owned(),
                vectors: vec![
                    entry(
                        "notes.txt_0",
                        "first",
                        "0",
                        Some("notes.txt"),
                        vec![0.5_f64, 0.25_f64]
                    ),
                    entry(
                        "notes.txt_1",
                        "second",
                        "1",
                        Some("notes.txt"),
                        vec![0.125_f64, 0.0625_f64]
                    ),
                ],
            },
        ]
    );
}

#[tokio::test]
async fn store_truncates_to_the_shorter_list() {
    let api = FakeApi::new();
    let workflow = rag_workflow(&api);
    let more_chunks = vec!["one".to_owned(), "two".to_owned(), "three".to_owned()];
    let two_vectors = vec![vec![1.0_f64], vec![2.0_f64]];
    workflow
        .store_vectors(Some("a.txt"), &more_chunks, Some(two_vectors.as_slice()))
        .await
        .unwrap();
    let one_chunk = vec!["one".to_owned()];
    workflow
        .store_vectors(Some("a.txt"), &one_chunk, Some(two_vectors.as_slice()))
        .await
        .unwrap();

    let calls = api.calls();
    let Call::Vectors { vectors, .. } = &calls[2] else {
        panic!("expected a put_vectors call")
    };
    assert_eq!(vectors.len(), 2);
    let Call::Vectors {
        vectors: truncated, ..
    } = &calls[3]
    else {
        panic!("expected a put_vectors call")
    };
    assert_eq!(truncated.len(), 1);
}

#[tokio::test]
async fn store_without_a_filename_uses_chunk_keys() {
    let api = FakeApi::new();
    let workflow = rag_workflow(&api);
    let chunks = vec!["first".to_owned()];
    let embeddings = vec![vec![0.5_f64]];
    let (id, name) = workflow
        .store_vectors(None, &chunks, Some(embeddings.as_slice()))
        .await
        .unwrap();
    assert_eq!(id.as_deref(), Some("rag:idx"));
    assert_eq!(name, None);
    assert_eq!(
        api.calls()[2],
        Call::Vectors {
            bucket_name: "rag".to_owned(),
            index_name: "idx".to_owned(),
            vectors: vec![entry("chunk_0", "first", "0", None, vec![0.5_f64])],
        }
    );
}

#[tokio::test]
async fn store_treats_an_empty_filename_as_absent() {
    let api = FakeApi::new();
    let workflow = rag_workflow(&api);
    let chunks = vec!["first".to_owned()];
    let embeddings = vec![vec![0.5_f64]];
    let (_, name) = workflow
        .store_vectors(Some(""), &chunks, Some(embeddings.as_slice()))
        .await
        .unwrap();
    assert_eq!(name.as_deref(), Some(""));
    assert_eq!(
        api.calls()[2],
        Call::Vectors {
            bucket_name: "rag".to_owned(),
            index_name: "idx".to_owned(),
            vectors: vec![entry("chunk_0", "first", "0", None, vec![0.5_f64])],
        }
    );
}

#[tokio::test]
async fn store_rejects_missing_content_after_initializing() {
    let api = FakeApi::new();
    let workflow = S3VectorsWorkflow::new(
        s3_settings(r#"{"vector_bucket_name": "b"}"#),
        api.clone(),
        FakeEmbedder::new(Vec::new(), false),
        UNIQUE_ID.to_owned(),
    );
    let chunks = vec!["one".to_owned()];
    let embeddings = vec![vec![1.0_f64]];
    let no_chunks = workflow
        .store_vectors(Some("a.txt"), &[], Some(embeddings.as_slice()))
        .await
        .unwrap_err();
    assert_eq!(no_chunks.to_string(), NO_CONTENT_MESSAGE);
    let no_embeddings = workflow
        .store_vectors(Some("a.txt"), &chunks, None)
        .await
        .unwrap_err();
    assert_eq!(no_embeddings.to_string(), NO_CONTENT_MESSAGE);
    let empty_embeddings = workflow
        .store_vectors(Some("a.txt"), &chunks, Some(&[]))
        .await
        .unwrap_err();
    assert_eq!(empty_embeddings.to_string(), NO_CONTENT_MESSAGE);

    assert_eq!(
        api.calls(),
        vec![
            bucket_call("b"),
            index_call("b", "litellm-index-abcd1234", DEFAULT_DIMENSION),
        ]
    );
}

#[tokio::test]
async fn store_trait_impl_forwards_the_columns() {
    let api = FakeApi::new();
    let workflow = rag_workflow(&api);
    let chunks = vec!["first".to_owned()];
    let embeddings = vec![vec![0.5_f64]];
    let request = StoreRequest {
        file_content: Some(b"ignored bytes"),
        filename: Some("notes.txt"),
        content_type: Some("text/plain"),
        chunks: &chunks,
        embeddings: Some(embeddings.as_slice()),
        existing_file_id: Some("file-1"),
    };
    let (id, name) = RagStore::store(&workflow, request).await.unwrap();
    assert_eq!(id.as_deref(), Some("rag:idx"));
    assert_eq!(name.as_deref(), Some("notes.txt"));
    assert_eq!(
        api.calls()[2],
        Call::Vectors {
            bucket_name: "rag".to_owned(),
            index_name: "idx".to_owned(),
            vectors: vec![entry(
                "notes.txt_0",
                "first",
                "0",
                Some("notes.txt"),
                vec![0.5_f64]
            )],
        }
    );
}

#[tokio::test]
async fn store_signs_every_request_through_the_bedrock_clients() {
    let (transport, resolver, id) = store_through_the_signed_api().await;
    assert_eq!(id.as_deref(), Some("sign-bucket:idx"));

    let seen = resolver.calls();
    assert_eq!(seen.len(), 5);
    assert!(seen.iter().all(|(settings, region)| {
        region == "eu-central-1"
            && settings.region_name.as_deref() == Some("eu-central-1")
            && settings.access_key_id.as_ref().unwrap().expose_secret() == "AKIDS3VECTORSLIVE"
            && settings.secret_access_key.as_ref().unwrap().expose_secret() == "vectors-live-secret"
    }));

    let sent = transport.sent();
    assert_eq!(sent.len(), 5);
    assert_eq!(
        sent[0].url,
        "https://s3vectors.eu-central-1.api.aws/GetVectorBucket"
    );
    assert_eq!(
        sent[4].url,
        "https://s3vectors.eu-central-1.api.aws/PutVectors"
    );
    let authorizations: Vec<&str> = sent
        .iter()
        .filter_map(|request| {
            request
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                .map(|(_, value)| value.as_str())
        })
        .collect();
    assert_eq!(authorizations.len(), 5);
    assert!(authorizations.iter().all(|value| {
        value.contains("Credential=AKIDS3VECTORSLIVE/")
            && value.contains("/eu-central-1/s3vectors/aws4_request")
    }));

    let body: PutVectorsBody = serde_json::from_slice(&sent[4].body).unwrap();
    assert_eq!(body.index_name, "idx");
    assert_eq!(
        body.vectors,
        vec![entry(
            "a.txt_0",
            "one",
            "0",
            Some("a.txt"),
            vec![0.5_f64, 0.25_f64]
        )]
    );
}

async fn store_through_the_signed_api() -> (RecordingTransport, StaticResolver, Option<String>) {
    let settings = s3_settings(
        r#"{"vector_bucket_name": "sign-bucket", "index_name": "idx", "dimension": 2, "aws_region_name": "eu-central-1", "aws_access_key_id": "AKIDS3VECTORSLIVE", "aws_secret_access_key": "vectors-live-secret"}"#,
    );
    let transport = RecordingTransport::answering(vec![
        not_found(),
        accepted(),
        not_found(),
        accepted(),
        accepted(),
    ]);
    let resolver = StaticResolver::default();
    let api = SignedS3Vectors::<'static, _>::new(
        &settings,
        Box::new(resolver.clone()),
        Box::new(FixedClock),
        transport.clone(),
    );
    let workflow = S3VectorsWorkflow::new(
        settings,
        api,
        FakeEmbedder::new(Vec::new(), false),
        UNIQUE_ID.to_owned(),
    );
    let chunks = vec!["one".to_owned()];
    let embeddings = vec![vec![0.5_f64, 0.25_f64]];
    let (id, _) = workflow
        .store_vectors(Some("a.txt"), &chunks, Some(embeddings.as_slice()))
        .await
        .unwrap();
    (transport, resolver, id)
}
