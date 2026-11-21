use betterlite_config::sections::vertex_credential_sources::VertexSettings;

use crate::rag::ingestion::IngestionState;

use super::super::VertexIngestion;
use super::harness::{File, build, store};
use super::{EXISTING_OPTIONS, FILE_REPLY};

#[test]
fn flags_supports_existing_file_id_matches_the_shared_state() {
    let supported = VertexIngestion::SUPPORTS_EXISTING_FILE_ID;
    assert!(!supported);
    assert_eq!(supported, IngestionState::SUPPORTS_EXISTING_FILE_ID);
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
    let (_, skipped_file_id) = store(&idle, idle_file).await.expect("a skipped upload");
    assert_eq!(skipped_file_id, None);
}
