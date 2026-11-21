use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use rstest::rstest;

use super::super::error::VertexIngestionError;
use super::super::upload::upload_file;
use super::harness::{File, build, settings, store};
use super::{FILE_REPLY, PROJECT_OPTIONS};

#[rstest]
#[case::parent("../other")]
#[case::query("proj?x=1")]
#[case::fragment("proj#frag")]
#[case::encoded_parent("projects/%2e%2e")]
#[tokio::test]
async fn project_unsafe_project_fails_before_any_port_call(#[case] project: &str) {
    let options = format!(r#"{{"vertex_project":"{project}"}}"#);
    let harness = build(&options, &VertexSettings::default(), None, None, &[]);
    let error = store(&harness, File::default())
        .await
        .expect_err("an unsafe project");
    assert!(matches!(error, VertexIngestionError::Location(_)));
    assert_eq!(
        error.to_string(),
        format!("the vertex_project '{project}' cannot be used in a URL")
    );
    assert!(harness.token.credentials_seen().is_empty());
    assert!(harness.http.calls().is_empty());
}

#[tokio::test]
async fn project_unsafe_alias_fails_before_any_port_call() {
    let options = r#"{"vertex_ai_project":"proj#frag"}"#;
    let harness = build(options, &VertexSettings::default(), None, None, &[]);
    let error = store(&harness, File::default())
        .await
        .expect_err("an unsafe project");
    assert!(matches!(error, VertexIngestionError::Location(_)));
    assert!(harness.token.credentials_seen().is_empty());
    assert!(harness.http.calls().is_empty());
}

#[tokio::test]
async fn project_unsafe_settings_project_fails_before_any_port_call() {
    let harness = build(
        "{}",
        &settings(Some("proj?x=1"), None, None),
        None,
        None,
        &[],
    );
    let error = store(&harness, File::default())
        .await
        .expect_err("an unsafe project");
    assert!(matches!(error, VertexIngestionError::Location(_)));
    assert!(harness.token.credentials_seen().is_empty());
    assert!(harness.http.calls().is_empty());
}

#[tokio::test]
async fn project_unsafe_project_fails_with_an_existing_corpus() {
    let options = r#"{"vertex_project":"proj?x=1","vector_store_id":"corpora/7"}"#;
    let harness = build(options, &VertexSettings::default(), None, None, &[]);
    let error = store(&harness, File::new(b"data", "notes.txt"))
        .await
        .expect_err("an unsafe project");
    assert!(matches!(error, VertexIngestionError::Location(_)));
    assert!(harness.token.credentials_seen().is_empty());
    assert!(harness.http.calls().is_empty());
}

#[rstest]
#[case::resource_name("corpora/7", "corpora/7")]
#[case::full_name(
    "projects/p/locations/l/ragCorpora/123",
    "projects/p/locations/l/ragCorpora/123"
)]
#[case::question("corpora/7?x=1", "corpora/7%3Fx%3D1")]
#[case::fragment("corpora/7#frag", "corpora/7%23frag")]
#[case::percent("corpora/7%2e%2e", "corpora/7%252e%252e")]
#[tokio::test]
async fn upload_encodes_the_corpus_id_in_the_path(#[case] corpus_id: &str, #[case] encoded: &str) {
    let options = format!(r#"{{"vertex_project":"proj","vector_store_id":"{corpus_id}"}}"#);
    let harness = build(
        &options,
        &VertexSettings::default(),
        None,
        None,
        &[(200, FILE_REPLY)],
    );
    store(&harness, File::new(b"data", "notes.txt"))
        .await
        .expect("an uploaded file");
    assert_eq!(
        harness.http.calls()[0].url,
        format!(
            "https://us-central1-aiplatform.googleapis.com/upload/v1beta1/{encoded}/ragFiles:upload"
        )
    );
}

#[rstest]
#[case::empty_segment("corpora//7", "vector_store_id is required")]
#[case::dot_segment("corpora/../7", "vector_store_id cannot be a dot path segment")]
#[case::dot("corpora/.", "vector_store_id cannot be a dot path segment")]
#[tokio::test]
async fn upload_rejects_an_unusable_corpus_id_before_any_port_call(
    #[case] corpus_id: &str,
    #[case] message: &str,
) {
    let options = format!(r#"{{"vertex_project":"proj","vector_store_id":"{corpus_id}"}}"#);
    let harness = build(&options, &VertexSettings::default(), None, None, &[]);
    let error = store(&harness, File::new(b"data", "notes.txt"))
        .await
        .expect_err("an unusable corpus id");
    assert!(matches!(error, VertexIngestionError::CorpusId(_)));
    assert_eq!(error.to_string(), message);
    assert!(harness.token.credentials_seen().is_empty());
    assert!(harness.http.calls().is_empty());
}

#[tokio::test]
async fn upload_rejects_an_empty_corpus_id_before_any_port_call() {
    let harness = build(PROJECT_OPTIONS, &VertexSettings::default(), None, None, &[]);
    let error = upload_file(&harness.ingestion, "", "notes.txt", b"data", None)
        .await
        .expect_err("an empty corpus id");
    assert!(matches!(error, VertexIngestionError::CorpusId(_)));
    assert_eq!(error.to_string(), "vector_store_id is required");
    assert!(harness.token.credentials_seen().is_empty());
    assert!(harness.http.calls().is_empty());
}

#[tokio::test]
async fn upload_encodes_the_created_corpus_id() {
    let created = r#"{"done":true,"response":{"name":"corpora/1?x=1"}}"#;
    let harness = build(
        PROJECT_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, created), (200, FILE_REPLY)],
    );
    store(&harness, File::new(b"data", "notes.txt"))
        .await
        .expect("an uploaded file");
    let calls = harness.http.calls();
    assert_eq!(
        calls[1].url,
        "https://us-central1-aiplatform.googleapis.com/upload/v1beta1/corpora/1%3Fx%3D1/ragFiles:upload"
    );
}
