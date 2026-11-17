#![cfg(test)]

use std::time::Duration;

use betterlite_config::rag::document::ingest::VertexAIVectorStoreOptions;
use betterlite_openai::files::records::CreatePurpose;
use rstest::rstest;
use secrecy::SecretString;

use crate::vertex_rag_ingest::chunking::{
    ChunkingConfig, ChunkingRecord, RagChunkingStrategy, SEPARATORS_IGNORED_WARNING,
};
use crate::vertex_rag_ingest::error::RagIngestError;
use crate::vertex_rag_ingest::execution::{
    DEFAULT_CONTENT_TYPE, RagIngest, StoreOutcome, StoreRequest,
};
use crate::vertex_rag_ingest::options::{
    DEFAULT_IMPORT_TIMEOUT, DEFAULT_LOCATION, RagIngestOptions, RagIngestSettings,
};
use crate::vertex_rag_ingest::tests::logs::install;

mod sdk;
mod support;

use self::support::{
    BUCKET, CORPUS_ID, CORPUS_NAME, EMPTY_CONTENT_WARNING, GCS_URI, ImportBehavior, Imported,
    PROJECT, Step, Uploaded, corpus_record, options, provider, resolve, store_request,
    vertex_settings,
};

#[test]
fn missing_corpus() {
    let error = resolve(&options(VertexAIVectorStoreOptions::default(), None)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "vector_store_id (corpus ID) is required for Vertex AI RAG ingestion. Please provide an existing RAG corpus ID."
    );
}

#[test]
fn the_other_required_fields() {
    let mut without_bucket = corpus_record();
    without_bucket.vertex_project = Some(PROJECT.to_owned());
    let bucket = RagIngestSettings::resolve(
        &options(without_bucket, None),
        &vertex_settings(None, None),
        None,
    )
    .unwrap_err();
    assert_eq!(
        bucket.to_string(),
        "gcs_bucket is required for Vertex AI RAG ingestion. Set via vector_store config or GCS_BUCKET_NAME env var."
    );

    let mut without_project = corpus_record();
    without_project.gcs_bucket = Some(BUCKET.to_owned());
    let project = RagIngestSettings::resolve(
        &options(without_project, None),
        &vertex_settings(None, None),
        Some(BUCKET),
    )
    .unwrap_err();
    assert_eq!(
        project.to_string(),
        "vertex_project is required for Vertex AI RAG ingestion. Set via vector_store config or VERTEXAI_PROJECT env var."
    );
}

#[test]
fn an_empty_bucket_environment_counts_as_unset() {
    let mut record = corpus_record();
    record.vertex_project = Some(PROJECT.to_owned());
    let error = RagIngestSettings::resolve(
        &options(record, None),
        &vertex_settings(None, None),
        Some(""),
    )
    .unwrap_err();
    assert!(matches!(error, RagIngestError::MissingBucket));
}

#[test]
fn settings_fall_back_to_the_vertex_environment() {
    let settings = RagIngestSettings::resolve(
        &options(corpus_record(), None),
        &vertex_settings(Some("b1301-project"), Some("europe-west4")),
        Some(BUCKET),
    )
    .unwrap();
    assert_eq!(settings.project, "b1301-project");
    assert_eq!(settings.location, "europe-west4");
    assert_eq!(settings.bucket, BUCKET);
    assert_eq!(
        settings.corpus_name,
        "projects/b1301-project/locations/europe-west4/ragCorpora/corpus"
    );
    assert!(settings.credentials.is_none());
    assert!(settings.wait_for_import);
    assert_eq!(settings.import_timeout, DEFAULT_IMPORT_TIMEOUT);
}

#[test]
fn the_location_defaults_to_us_central_one() {
    let settings = resolve(&options(corpus_record(), None)).unwrap();
    assert_eq!(settings.location, DEFAULT_LOCATION);
    assert_eq!(settings.corpus_name, CORPUS_NAME);
}

#[test]
fn explicit_import_controls_are_kept() {
    let mut record = corpus_record();
    record.wait_for_import = Some(false);
    record.import_timeout = Some(0);
    let settings = resolve(&options(record, None)).unwrap();
    assert!(!settings.wait_for_import);
    assert_eq!(settings.import_timeout, Duration::ZERO);
}

#[test]
fn the_ingest_document_is_read_through_the_provider_options() {
    let options: RagIngestOptions = serde_json::from_str(
        r#"{"name":"ignored","chunking_strategy":{"chunk_size":0},"vector_store":{"custom_llm_provider":"vertex_ai","vector_store_id":"corpus","gcs_bucket":"bucket","vertex_project":"project","wait_for_import":false,"import_timeout":0}}"#,
    )
    .expect("the ingest document is valid");
    let settings =
        RagIngestSettings::resolve(&options, &vertex_settings(None, None), None).unwrap();
    assert_eq!(settings.corpus_name, CORPUS_NAME);
    assert_eq!(settings.bucket, BUCKET);
    assert!(!settings.wait_for_import);
    assert_eq!(settings.import_timeout, Duration::ZERO);
    assert_eq!(
        settings
            .chunking_strategy
            .expect("the strategy parses")
            .chunk_size,
        Some(0)
    );
}

#[rstest]
#[case::absent(None, None)]
#[case::empty_content(Some(b"".as_slice()), Some("report.txt"))]
#[case::empty_filename(Some(b"data".as_slice()), Some(""))]
#[tokio::test]
async fn empty_source_warning(#[case] content: Option<&[u8]>, #[case] filename: Option<&str>) {
    let (_guard, logs) = install();
    let (ingest, recorder) = provider(corpus_record(), None, ImportBehavior::Imported(1));
    let outcome = ingest
        .store(StoreRequest {
            file_content: content,
            filename,
            content_type: None,
            chunks: &[],
            embeddings: None,
            existing_file_id: None,
        })
        .await
        .unwrap();
    assert_eq!(
        outcome,
        StoreOutcome {
            corpus_id: Some(CORPUS_ID.to_owned()),
            file_id: None,
        }
    );
    assert_eq!(logs.warnings(), vec![EMPTY_CONTENT_WARNING.to_owned()]);
    assert_eq!(recorder.steps(), Vec::new());
}

#[tokio::test]
async fn upload_then_import() {
    let (_guard, logs) = install();
    let (ingest, recorder) = provider(corpus_record(), None, ImportBehavior::Imported(1));
    let outcome = ingest
        .store(store_request(b"hello", "report.txt", None))
        .await
        .unwrap();
    assert_eq!(
        (outcome.corpus_id.as_deref(), outcome.file_id.as_deref()),
        (Some(CORPUS_ID), Some(GCS_URI))
    );
    assert_eq!(recorder.steps(), vec![Step::Upload, Step::Import]);
    assert_eq!(
        recorder.upload().content_type.as_deref(),
        Some(DEFAULT_CONTENT_TYPE)
    );
    assert_eq!(
        recorder.imported(),
        Imported {
            corpus: CORPUS_NAME.to_owned(),
            gcs_uri: GCS_URI.to_owned(),
            chunking: ChunkingRecord {
                chunking_config: ChunkingConfig {
                    chunk_size: 1000,
                    chunk_overlap: 200,
                },
            },
        }
    );
    assert!(
        logs.infos()
            .contains(&"Import complete: 1 files imported".to_owned())
    );
}

#[tokio::test]
async fn supplied_chunks_and_embeddings_are_ignored() {
    let (ingest, recorder) = provider(corpus_record(), None, ImportBehavior::Imported(1));
    let chunks = vec!["chunk".to_owned()];
    let embeddings = vec![vec![0.25_f32, 0.75_f32]];
    let outcome = ingest
        .store(StoreRequest {
            file_content: Some(b"hello"),
            filename: Some("report.txt"),
            content_type: None,
            chunks: &chunks,
            embeddings: Some(&embeddings),
            existing_file_id: Some("existing-file"),
        })
        .await
        .unwrap();
    assert_eq!(outcome.file_id.as_deref(), Some(GCS_URI));
    assert_eq!(recorder.upload().data, b"hello");
    assert_eq!(recorder.steps(), vec![Step::Upload, Step::Import]);
}

#[tokio::test]
async fn upload_identity_vs_adc() {
    let mut with_credentials = corpus_record();
    with_credentials.vertex_credentials = Some(SecretString::from("service-account.json"));
    let (without, without_recorder) = provider(corpus_record(), None, ImportBehavior::Imported(1));
    let (with, with_recorder) = provider(with_credentials, None, ImportBehavior::Imported(1));
    let first = without
        .store(store_request(b"hello", "report.txt", Some("text/plain")))
        .await
        .unwrap();
    let second = with
        .store(store_request(b"hello", "report.txt", Some("text/plain")))
        .await
        .unwrap();
    assert_eq!(
        without_recorder.upload(),
        Uploaded {
            filename: Some("report.txt".to_owned()),
            content_type: Some("text/plain".to_owned()),
            data: b"hello".to_vec(),
            purpose: CreatePurpose::Assistants,
            bucket: BUCKET.to_owned(),
            project: PROJECT.to_owned(),
            location: DEFAULT_LOCATION.to_owned(),
            credentials: None,
        }
    );
    assert_eq!(
        with_recorder.upload().credentials.as_deref(),
        Some("service-account.json")
    );
    assert_eq!(without_recorder.imported(), with_recorder.imported());
    assert_eq!(first, second);
}

#[tokio::test]
async fn import_error_keeps_file() {
    let (_guard, logs) = install();
    let (ingest, recorder) = provider(corpus_record(), None, ImportBehavior::Rejected);
    let error = ingest
        .store(store_request(b"hello", "report.txt", None))
        .await
        .unwrap_err();
    let RagIngestError::Import { source } = &error else {
        panic!("a rejected import is an import error");
    };
    assert!(source.to_string().contains("the corpus rejected the file"));
    let message = error.to_string();
    assert!(message.starts_with("Failed to import file into RAG corpus: "));
    assert!(message.contains("the corpus rejected the file"));
    assert_eq!(recorder.steps(), vec![Step::Upload, Step::Import]);
    assert_eq!(recorder.retained(), vec![GCS_URI.to_owned()]);
    assert!(
        logs.errors()
            .iter()
            .any(|line| line.starts_with("Failed to import file into RAG corpus: "))
    );
}

#[tokio::test]
async fn false_wait_no_rpc() {
    let (_guard, logs) = install();
    let mut record = corpus_record();
    record.wait_for_import = Some(false);
    let strategy: RagChunkingStrategy =
        serde_json::from_str(r#"{"chunk_size": 10, "separators": ["\n"]}"#)
            .expect("the strategy is valid");
    let (ingest, recorder) = provider(record, Some(strategy), ImportBehavior::Rejected);
    let outcome = ingest
        .store(store_request(b"hello", "report.txt", None))
        .await
        .unwrap();
    assert_eq!(outcome.file_id.as_deref(), Some(GCS_URI));
    assert_eq!(recorder.steps(), vec![Step::Upload]);
    assert_eq!(logs.warnings(), vec![SEPARATORS_IGNORED_WARNING.to_owned()]);
    let infos = logs.infos();
    assert!(infos.contains(&"Import started asynchronously".to_owned()));
    assert!(!infos.iter().any(|line| line.starts_with("Import complete")));
}

#[rstest]
#[case::default_timeout(None, "the import timed out after 600 seconds")]
#[case::explicit_timeout(Some(5), "the import timed out after 5 seconds")]
#[tokio::test(start_paused = true)]
async fn import_timeout(#[case] import_timeout: Option<i64>, #[case] message: &str) {
    let (_guard, logs) = install();
    let mut record = corpus_record();
    record.import_timeout = import_timeout;
    let (ingest, recorder) = provider(record, None, ImportBehavior::NeverFinishes);
    let error = ingest
        .store(store_request(b"hello", "report.txt", None))
        .await
        .unwrap_err();
    let RagIngestError::Import { source } = &error else {
        panic!("a pending import is an import timeout");
    };
    assert_eq!(source.to_string(), message);
    assert_eq!(
        error.to_string(),
        format!("Failed to import file into RAG corpus: {message}")
    );
    assert_eq!(recorder.steps(), vec![Step::Upload, Step::Import]);
    assert!(
        logs.errors()
            .iter()
            .any(|line| line.starts_with("Failed to import file into RAG corpus: "))
    );
}
