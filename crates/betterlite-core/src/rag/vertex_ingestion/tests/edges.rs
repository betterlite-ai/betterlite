use betterlite_config::sections::vertex_credential_sources::VertexSettings;

use super::EXISTING_OPTIONS;
use super::harness::{File, build, store};

#[tokio::test]
async fn upload_null_rag_file_parts_fall_back() {
    let named = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, r#"{"ragFile":{"name":null},"name":"top"}"#)],
    );
    let (_, file_id) = store(&named, File::new(b"data", "notes.txt"))
        .await
        .expect("an uploaded file");
    assert_eq!(file_id.as_deref(), Some("top"));
    let uploaded = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[(200, r#"{"ragFile":null,"name":"top"}"#)],
    );
    let (_, null_part_file_id) = store(&uploaded, File::new(b"data", "notes.txt"))
        .await
        .expect("an uploaded file");
    assert_eq!(null_part_file_id.as_deref(), Some("uploaded"));
}

#[tokio::test]
async fn error_http_port_failure_carries_the_lower_message() {
    let harness = build(
        EXISTING_OPTIONS,
        &VertexSettings::default(),
        None,
        None,
        &[],
    );
    let error = store(&harness, File::new(b"data", "notes.txt"))
        .await
        .expect_err("a port failure");
    assert_eq!(error.to_string(), "the reply script is exhausted");
    assert_eq!(harness.http.calls().len(), 1);
    assert_eq!(harness.token.credentials_seen(), [false]);
}
