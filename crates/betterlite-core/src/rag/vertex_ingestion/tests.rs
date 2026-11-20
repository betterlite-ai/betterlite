#![cfg(test)]

mod harness;

use std::time::Duration;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use rstest::rstest;

use crate::rag::ingestion::IngestionState;

use super::VertexIngestion;

use harness::{File, Kind, build, capture, chunking, expected, json_body, settings, store};

const TOKEN: &str = "tok-123";
const PROJECT_OPTIONS: &str = r#"{"vertex_project":"proj"}"#;
const EXISTING_OPTIONS: &str = r#"{"vertex_project":"proj","vector_store_id":"corpora/7"}"#;
const NOT_DONE: &str = r#"{"done":false,"name":"operations/1"}"#;
const DONE: &str = r#"{"done":true,"response":{"name":"corpora/1"}}"#;
const FILE_REPLY: &str = r#"{"ragFile":{"name":"files/1"}}"#;
const CORPUS_URL: &str = "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/us-central1/ragCorpora";
const UPLOAD_URL: &str =
    "https://us-central1-aiplatform.googleapis.com/upload/v1beta1/corpora/7/ragFiles:upload";

#[test]
fn flags_supports_existing_file_id_matches_the_shared_state() {
    assert!(!VertexIngestion::SUPPORTS_EXISTING_FILE_ID);
    assert_eq!(
        VertexIngestion::SUPPORTS_EXISTING_FILE_ID,
        IngestionState::SUPPORTS_EXISTING_FILE_ID
    );
}

#[tokio::test]
async fn flags_embed_returns_no_embeddings() {
    let (logs, _guard) = capture();
    let harness = build(PROJECT_OPTIONS, &VertexSettings::default(), None, None, &[]);
    let chunks = ["one chunk".to_owned()];
    assert_eq!(harness.ingestion.embed(&chunks).await, None);
    assert!(
        logs.text()
            .contains("Skipping the embedding step for 1 chunks")
    );
}

#[tokio::test]
async fn flags_existing_file_id_is_ignored() {
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, FILE_REPLY)],
    );
    let file = File {
        content: Some(b"data".as_slice()),
        filename: Some("n.txt"),
        content_type: None,
        existing_file_id: Some("legacy"),
    };
    let (_, file_id) = store(&harness, file).await.expect("a stored file");
    assert_eq!(file_id.as_deref(), Some("files/1"));
    let idle = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[],
    );
    let idle_file = File {
        content: None,
        filename: None,
        content_type: None,
        existing_file_id: Some("legacy"),
    };
    let (_, file_id) = store(&idle, idle_file).await.expect("a skipped upload");
    assert_eq!(file_id, None);
}

#[tokio::test]
async fn project_missing_project_fails_without_any_port_call() {
    let harness = build("{}", &VertexSettings::default(), None, None, &[]);
    let error = store(&harness, File::default())
        .await
        .expect_err("a missing project");
    assert_eq!(
        error.to_string(),
        "vertex_project is required for Vertex AI RAG ingestion. Set it in vector_store config."
    );
    assert!(harness.token.credentials_seen().is_empty());
    assert!(harness.http.calls().is_empty());
}

#[tokio::test]
async fn project_empty_project_option_is_missing() {
    let harness = build(
        r#"{"vertex_project":""}"#,
        &settings(Some("settings-proj"), None),
        None,
        None,
        &[],
    );
    let error = store(&harness, File::default())
        .await
        .expect_err("an empty project");
    assert_eq!(
        error.to_string(),
        "vertex_project is required for Vertex AI RAG ingestion. Set it in vector_store config."
    );
    assert!(harness.token.credentials_seen().is_empty());
}

#[tokio::test]
async fn project_settings_project_is_used_when_the_options_omit_one() {
    let harness = build(
        "{}",
        &settings(Some("settings-proj"), None),
        None,
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        harness.http.calls()[0].url,
        "https://us-central1-aiplatform.googleapis.com/v1beta1/projects/settings-proj/locations/us-central1/ragCorpora"
    );
}

#[tokio::test]
async fn project_vertex_project_wins_over_the_vertex_ai_alias() {
    let options = r#"{"vertex_project":"primary","vertex_ai_project":"alias"}"#;
    let harness = build(
        options,
        &settings(Some("settings-proj"), None),
        None,
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert!(
        harness.http.calls()[0]
            .url
            .contains("/projects/primary/locations/")
    );
}

#[tokio::test]
async fn project_location_defaults_to_us_central1() {
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(harness.http.calls()[0].url, CORPUS_URL);
}

#[tokio::test]
async fn project_location_uses_the_settings_value() {
    let harness = build(
        PROJECT_OPTIONS,
        &settings(None, Some("europe-west4")),
        None,
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        harness.http.calls()[0].url,
        "https://europe-west4-aiplatform.googleapis.com/v1beta1/projects/proj/locations/europe-west4/ragCorpora"
    );
}

#[tokio::test]
async fn project_location_aliases_fall_through_empties() {
    let options =
        r#"{"vertex_project":"proj","vertex_location":"","vertex_ai_location":"asia-east1"}"#;
    let harness = build(
        options,
        &VertexSettings::default(),
        None,
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        harness.http.calls()[0].url,
        "https://asia-east1-aiplatform.googleapis.com/v1beta1/projects/proj/locations/asia-east1/ragCorpora"
    );
}

#[tokio::test]
async fn project_location_uses_the_global_host() {
    let options = r#"{"vertex_project":"proj","vertex_location":"global"}"#;
    let harness = build(
        options,
        &VertexSettings::default(),
        None,
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        harness.http.calls()[0].url,
        "https://aiplatform.googleapis.com/v1beta1/projects/proj/locations/global/ragCorpora"
    );
}

#[tokio::test]
async fn project_location_uses_the_multiregion_host() {
    let options = r#"{"vertex_project":"proj","vertex_location":"eu"}"#;
    let harness = build(
        options,
        &VertexSettings::default(),
        None,
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        harness.http.calls()[0].url,
        "https://aiplatform.eu.rep.googleapis.com/v1beta1/projects/proj/locations/eu/ragCorpora"
    );
}

#[tokio::test]
async fn project_invalid_location_fails_after_the_token_call() {
    let options = r#"{"vertex_project":"proj","vertex_location":"Bad_Location"}"#;
    let harness = build(options, &VertexSettings::default(), None, None, &[]);
    let error = store(&harness, File::default())
        .await
        .expect_err("an invalid location");
    assert_eq!(error.to_string(), "invalid vertex_location format");
    assert_eq!(harness.token.credentials_seen().len(), 1);
    assert!(harness.http.calls().is_empty());
}

#[tokio::test]
async fn corpus_create_sends_the_camel_case_body() {
    let options =
        r#"{"vertex_project":"proj","description":"notes","vector_db_config":{"index":"x"}}"#;
    let harness = build(
        options,
        &VertexSettings::default(),
        Some("ingest-name"),
        None,
        &[(200, DONE)],
    );
    let (corpus, file_id) = store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(corpus.as_deref(), Some("corpora/1"));
    assert_eq!(file_id, None);
    let calls = harness.http.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].kind, Kind::Json);
    assert_eq!(calls[0].url, CORPUS_URL);
    assert_eq!(calls[0].timeout, Duration::from_secs(60));
    assert_eq!(
        json_body(&calls[0]),
        expected(
            r#"{"displayName":"ingest-name","description":"notes","vectorDbConfig":{"index":"x"}}"#
        )
    );
    assert_eq!(harness.token.credentials_seen().len(), 1);
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[tokio::test]
async fn corpus_create_defaults_the_display_name(#[case] name: Option<&str>) {
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        name,
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        json_body(&harness.http.calls()[0]),
        expected(r#"{"displayName":"litellm-rag-corpus"}"#)
    );
}

#[tokio::test]
async fn corpus_create_drops_an_empty_description() {
    let options = r#"{"vertex_project":"proj","description":""}"#;
    let harness = build(
        options,
        &VertexSettings::default(),
        Some("named"),
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        json_body(&harness.http.calls()[0]),
        expected(r#"{"displayName":"named"}"#)
    );
}

#[tokio::test]
async fn corpus_create_merges_the_embedding_model() {
    let options = r#"{"vertex_project":"proj","embedding_model":"text-embedding-005","vector_db_config":{"a":1,"ragEmbeddingModelConfig":{"keep":true}}}"#;
    let harness = build(
        options,
        &VertexSettings::default(),
        None,
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        json_body(&harness.http.calls()[0]),
        expected(
            r#"{"displayName":"litellm-rag-corpus","vectorDbConfig":{"a":1,"ragEmbeddingModelConfig":{"vertexPredictionEndpoint":{"endpoint":"text-embedding-005"}}}}"#
        )
    );
}

#[tokio::test]
async fn corpus_create_sends_vector_db_config_alone() {
    let options = r#"{"vertex_project":"proj","vector_db_config":{"a":1}}"#;
    let harness = build(
        options,
        &VertexSettings::default(),
        None,
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        json_body(&harness.http.calls()[0]),
        expected(r#"{"displayName":"litellm-rag-corpus","vectorDbConfig":{"a":1}}"#)
    );
}

#[tokio::test]
async fn corpus_create_drops_an_empty_vector_db_config() {
    let options = r#"{"vertex_project":"proj","vector_db_config":{},"embedding_model":""}"#;
    let harness = build(
        options,
        &VertexSettings::default(),
        None,
        None,
        &[(200, DONE)],
    );
    store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(
        json_body(&harness.http.calls()[0]),
        expected(r#"{"displayName":"litellm-rag-corpus"}"#)
    );
}

#[tokio::test]
async fn corpus_create_rejects_a_non_object_vector_db_config() {
    let options = r#"{"vertex_project":"proj","vector_db_config":"text"}"#;
    let harness = build(options, &VertexSettings::default(), None, None, &[]);
    let error = store(&harness, File::default())
        .await
        .expect_err("a vector db config error");
    assert_eq!(error.to_string(), "`vector_db_config` must be an object");
    assert!(harness.http.calls().is_empty());
}

#[tokio::test]
async fn corpus_create_done_operation_returns_the_corpus_name() {
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(201, DONE)],
    );
    let (corpus, file_id) = store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(corpus.as_deref(), Some("corpora/1"));
    assert_eq!(file_id, None);
    assert_eq!(harness.http.calls().len(), 1);
}

#[tokio::test]
async fn corpus_create_done_without_a_name_returns_an_empty_corpus() {
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, r#"{"done":true}"#)],
    );
    let (corpus, file_id) = store(&harness, File::new(b"data", "n.txt"))
        .await
        .expect("an empty corpus");
    assert_eq!(corpus.as_deref(), Some(""));
    assert_eq!(file_id, None);
    assert_eq!(harness.http.calls().len(), 1);
}

#[tokio::test]
async fn corpus_create_existing_corpus_skips_the_create() {
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[],
    );
    let (corpus, file_id) = store(&harness, File::default())
        .await
        .expect("an existing corpus");
    assert_eq!(corpus.as_deref(), Some("corpora/7"));
    assert_eq!(file_id, None);
    assert!(harness.http.calls().is_empty());
    assert!(harness.token.credentials_seen().is_empty());
}

#[tokio::test]
async fn corpus_create_treats_an_empty_vector_store_id_as_missing() {
    let options = r#"{"vertex_project":"proj","vector_store_id":""}"#;
    let harness = build(
        options,
        &VertexSettings::default(),
        None,
        None,
        &[(200, DONE)],
    );
    let (corpus, file_id) = store(&harness, File::default())
        .await
        .expect("a created corpus");
    assert_eq!(corpus.as_deref(), Some("corpora/1"));
    assert_eq!(file_id, None);
    assert_eq!(harness.http.calls().len(), 1);
    assert_eq!(harness.http.calls()[0].kind, Kind::Json);
}

#[tokio::test(start_paused = true)]
async fn poll_pending_operation_polls_until_done() {
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, NOT_DONE), (200, DONE)],
    );
    let started = tokio::time::Instant::now();
    let (corpus, _) = store(&harness, File::default())
        .await
        .expect("a polled corpus");
    assert_eq!(
        tokio::time::Instant::now() - started,
        Duration::from_secs(2)
    );
    assert_eq!(corpus.as_deref(), Some("corpora/1"));
    let calls = harness.http.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[1].kind, Kind::Get);
    assert_eq!(
        calls[1].url,
        "https://us-central1-aiplatform.googleapis.com/v1beta1/operations/1"
    );
    assert_eq!(calls[1].timeout, Duration::from_secs(60));
    assert_eq!(calls[1].token, TOKEN);
    assert_eq!(calls[2].kind, Kind::Get);
    assert_eq!(harness.token.credentials_seen().len(), 1);
    assert_eq!(harness.token.projects_seen(), [Some("proj".to_owned())]);
}

#[tokio::test(start_paused = true)]
async fn poll_operation_error_fails() {
    let replies = [
        (200, NOT_DONE),
        (200, r#"{"done":true,"error":{"code":5,"message":"boom"}}"#),
    ];
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &replies,
    );
    let error = store(&harness, File::default())
        .await
        .expect_err("a failed operation");
    assert_eq!(
        error.to_string(),
        r#"Operation failed: {"code":5,"message":"boom"}"#
    );
}

#[tokio::test(start_paused = true)]
async fn poll_null_operation_error_fails() {
    let replies = [(200, NOT_DONE), (200, r#"{"done":true,"error":null}"#)];
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &replies,
    );
    let error = store(&harness, File::default())
        .await
        .expect_err("a failed operation");
    assert_eq!(error.to_string(), "Operation failed: null");
}

#[tokio::test(start_paused = true)]
async fn poll_empty_corpus_name_fails() {
    let replies = [
        (200, NOT_DONE),
        (200, r#"{"done":true,"response":{"name":""}}"#),
    ];
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &replies,
    );
    let error = store(&harness, File::default())
        .await
        .expect_err("a nameless operation");
    assert_eq!(
        error.to_string(),
        r#"No corpus name in operation response: {"done":true,"response":{"name":""}}"#
    );
}

#[tokio::test(start_paused = true)]
async fn poll_non_200_status_fails() {
    let replies = [(200, NOT_DONE), (500, "unavailable")];
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &replies,
    );
    let error = store(&harness, File::default())
        .await
        .expect_err("a failed poll");
    assert_eq!(error.to_string(), "Failed to poll operation: unavailable");
}

#[tokio::test(start_paused = true)]
async fn poll_gives_up_after_thirty_attempts() {
    let replies = [(200_u16, NOT_DONE); 30];
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &replies,
    );
    let started = tokio::time::Instant::now();
    let error = store(&harness, File::default())
        .await
        .expect_err("a timeout");
    assert_eq!(error.to_string(), "Operation timed out after 30 attempts");
    assert_eq!(
        tokio::time::Instant::now() - started,
        Duration::from_secs(60)
    );
    assert_eq!(harness.http.calls().len(), 31);
    assert_eq!(harness.token.credentials_seen().len(), 1);
}

#[tokio::test]
async fn upload_posts_the_multipart_form() {
    let options =
        r#"{"vertex_project":"proj","vector_store_id":"corpora/7","file_description":""}"#;
    let harness = build(
        options,
        &VertexSettings::default(),
        None,
        None,
        &[(200, FILE_REPLY)],
    );
    let file = File {
        content: Some(b"data".as_slice()),
        filename: Some("notes.txt"),
        content_type: Some("text/plain"),
        existing_file_id: None,
    };
    let (corpus, file_id) = store(&harness, file).await.expect("an uploaded file");
    assert_eq!(corpus.as_deref(), Some("corpora/7"));
    assert_eq!(file_id.as_deref(), Some("files/1"));
    let calls = harness.http.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].kind, Kind::Multipart);
    assert_eq!(calls[0].url, UPLOAD_URL);
    assert_eq!(calls[0].token, TOKEN);
    assert_eq!(calls[0].filename, "notes.txt");
    assert_eq!(calls[0].file, b"data".to_vec());
    assert_eq!(calls[0].content_type, "text/plain");
    assert_eq!(calls[0].timeout, Duration::from_secs(300));
    assert_eq!(harness.token.projects_seen(), [Some("proj".to_owned())]);
    assert_eq!(
        json_body(&calls[0]),
        expected(r#"{"rag_file":{"display_name":"notes.txt"}}"#)
    );
}

#[tokio::test]
async fn upload_description_comes_from_file_description() {
    let options =
        r#"{"vertex_project":"proj","vector_store_id":"corpora/7","file_description":"described"}"#;
    let harness = build(
        options,
        &VertexSettings::default(),
        None,
        None,
        &[(200, FILE_REPLY)],
    );
    let file = File {
        content: Some(b"data".as_slice()),
        filename: Some("notes.txt"),
        content_type: Some(""),
        existing_file_id: None,
    };
    store(&harness, file).await.expect("an uploaded file");
    assert_eq!(
        json_body(&harness.http.calls()[0]),
        expected(r#"{"rag_file":{"display_name":"notes.txt","description":"described"}}"#)
    );
    assert_eq!(
        harness.http.calls()[0].content_type,
        "application/octet-stream"
    );
}

#[tokio::test]
async fn upload_chunking_config_carries_only_nonzero_sizes() {
    let strategy = chunking(&[("chunk_size", "500"), ("chunk_overlap", "0")]);
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        Some(strategy),
        &[(200, FILE_REPLY)],
    );
    store(&harness, File::new(b"data", "notes.txt"))
        .await
        .expect("an uploaded file");
    assert_eq!(
        json_body(&harness.http.calls()[0]),
        expected(
            r#"{"rag_file":{"display_name":"notes.txt"},"upload_rag_file_config":{"rag_file_transformation_config":{"rag_file_chunking_config":{"fixed_length_chunking":{"chunk_size":500}}}}}"#
        )
    );
}

#[tokio::test]
async fn upload_omits_the_chunking_config_for_zero_sizes() {
    let strategy = chunking(&[("chunk_size", "0"), ("chunk_overlap", "0")]);
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        Some(strategy),
        &[(200, FILE_REPLY)],
    );
    store(&harness, File::new(b"data", "notes.txt"))
        .await
        .expect("an uploaded file");
    assert_eq!(
        json_body(&harness.http.calls()[0]),
        expected(r#"{"rag_file":{"display_name":"notes.txt"}}"#)
    );
}

#[tokio::test]
async fn upload_skips_empty_content() {
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[],
    );
    let file = File::new(b"", "notes.txt");
    let (corpus, file_id) = store(&harness, file).await.expect("a skipped upload");
    assert_eq!(corpus.as_deref(), Some("corpora/7"));
    assert_eq!(file_id, None);
    assert!(harness.http.calls().is_empty());
    assert!(harness.token.credentials_seen().is_empty());
}

#[tokio::test]
async fn upload_skips_an_empty_filename() {
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[],
    );
    let file = File::new(b"data", "");
    let (_, file_id) = store(&harness, file).await.expect("a skipped upload");
    assert_eq!(file_id, None);
    assert!(harness.http.calls().is_empty());
}

#[tokio::test]
async fn upload_skips_an_empty_corpus_id() {
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, r#"{"done":true}"#)],
    );
    let (corpus, file_id) = store(&harness, File::new(b"data", "notes.txt"))
        .await
        .expect("an empty corpus");
    assert_eq!(corpus.as_deref(), Some(""));
    assert_eq!(file_id, None);
    assert_eq!(harness.http.calls()[0].kind, Kind::Json);
    assert_eq!(harness.http.calls().len(), 1);
}

#[tokio::test]
async fn upload_file_id_prefers_rag_file_name() {
    let reply = r#"{"ragFile":{"name":"files/1"},"name":"top"}"#;
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, reply)],
    );
    let (_, file_id) = store(&harness, File::new(b"data", "notes.txt"))
        .await
        .expect("an uploaded file");
    assert_eq!(file_id.as_deref(), Some("files/1"));
}

#[tokio::test]
async fn upload_file_id_falls_back_to_the_top_level_name() {
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(201, r#"{"name":"top"}"#)],
    );
    let (_, file_id) = store(&harness, File::new(b"data", "notes.txt"))
        .await
        .expect("an uploaded file");
    assert_eq!(file_id.as_deref(), Some("top"));
}

#[tokio::test]
async fn upload_file_id_is_empty_when_absent() {
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, "{}")],
    );
    let (_, file_id) = store(&harness, File::new(b"data", "notes.txt"))
        .await
        .expect("an uploaded file");
    assert_eq!(file_id.as_deref(), Some(""));
}

#[tokio::test]
async fn upload_unparsable_response_returns_uploaded() {
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, "not json")],
    );
    let (_, file_id) = store(&harness, File::new(b"data", "notes.txt"))
        .await
        .expect("an uploaded file");
    assert_eq!(file_id.as_deref(), Some("uploaded"));
}

#[tokio::test]
async fn upload_non_object_rag_file_returns_uploaded() {
    let reply = r#"{"ragFile":"text","name":"top"}"#;
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, reply)],
    );
    let (_, file_id) = store(&harness, File::new(b"data", "notes.txt"))
        .await
        .expect("an uploaded file");
    assert_eq!(file_id.as_deref(), Some("uploaded"));
}

#[tokio::test]
async fn error_create_status_failure_carries_the_body() {
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(400, "bad request")],
    );
    let error = store(&harness, File::default())
        .await
        .expect_err("a create failure");
    assert_eq!(
        error.to_string(),
        "Failed to create RAG corpus: bad request"
    );
    assert_eq!(harness.http.calls().len(), 1);
}

#[tokio::test]
async fn error_undecodable_body_bytes_are_replaced() {
    let harness = build(PROJECT_OPTIONS, &VertexSettings::default(), None, None, &[]);
    harness.http.push_reply(400, b"bad \xFF request");
    let error = store(&harness, File::default())
        .await
        .expect_err("a create failure");
    assert_eq!(
        error.to_string(),
        "Failed to create RAG corpus: bad \u{fffd} request"
    );
}

#[tokio::test(start_paused = true)]
async fn error_poll_status_failure_carries_the_body() {
    let replies = [(200, NOT_DONE), (503, "unavailable")];
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &replies,
    );
    let error = store(&harness, File::default())
        .await
        .expect_err("a poll failure");
    assert_eq!(error.to_string(), "Failed to poll operation: unavailable");
    assert_eq!(harness.http.calls().len(), 2);
}

#[tokio::test]
async fn error_upload_status_failure_keeps_the_created_corpus() {
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, DONE), (500, "boom")],
    );
    let error = store(&harness, File::new(b"data", "notes.txt"))
        .await
        .expect_err("an upload failure");
    assert_eq!(error.to_string(), "Failed to upload file: boom");
    let calls = harness.http.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].kind, Kind::Json);
    assert_eq!(
        json_body(&calls[0]),
        expected(r#"{"displayName":"litellm-rag-corpus"}"#)
    );
    assert_eq!(calls[1].kind, Kind::Multipart);
}

#[tokio::test]
async fn error_create_non_json_response_is_a_decode_error() {
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, "noise")],
    );
    let error = store(&harness, File::default())
        .await
        .expect_err("a decode failure");
    assert_eq!(
        error.to_string(),
        "the create corpus response is not valid JSON"
    );
}

#[tokio::test(start_paused = true)]
async fn error_poll_non_object_response_is_a_shape_error() {
    let replies = [(200, NOT_DONE), (200, r#""not an object""#)];
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &replies,
    );
    let error = store(&harness, File::default())
        .await
        .expect_err("a shape failure");
    assert_eq!(error.to_string(), "the poll response is not a JSON object");
}

#[tokio::test]
async fn error_non_boolean_done_is_a_field_error() {
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, r#"{"done":"yes"}"#)],
    );
    let error = store(&harness, File::default())
        .await
        .expect_err("a field failure");
    assert_eq!(error.to_string(), "`done` must be a boolean");
}

#[tokio::test]
async fn secret_token_never_reaches_the_logs() {
    let (logs, _guard) = capture();
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, FILE_REPLY)],
    );
    store(&harness, File::new(b"data", "notes.txt"))
        .await
        .expect("an uploaded file");
    let text = logs.text();
    assert!(text.contains("Uploading file to RAG corpus"));
    assert!(text.contains("Upload complete. File ID: files/1"));
    assert!(!text.contains(TOKEN));
}

#[tokio::test]
async fn secret_credentials_stay_out_of_logs_and_errors() {
    const SECRET: &str = "private-key-material";
    let (logs, _guard) = capture();
    let credentials = format!(
        r#"{{"type":"service_account","client_email":"sa@example.iam","private_key":"{SECRET}"}}"#
    );
    let options = format!(
        r#"{{"vertex_project":"proj","vertex_credentials":{}}}"#,
        serde_json::to_string(&credentials).expect("a JSON string")
    );
    let harness = build(
        &options,
        &VertexSettings::default(),
        None,
        None,
        &[(200, DONE), (500, "boom")],
    );
    let error = store(&harness, File::new(b"data", "notes.txt"))
        .await
        .expect_err("an upload failure");
    assert_eq!(harness.token.credentials_seen(), vec![true]);
    assert!(!logs.text().contains(SECRET));
    assert!(!error.to_string().contains(SECRET));
    assert!(!format!("{error:?}").contains(SECRET));
}
