#![cfg(test)]

mod codec;
mod support;
mod transport;

use std::time::Duration;

use http::HeaderMap;
use http::header::HeaderValue;
use secrecy::SecretString;
use tracing::Level;

use self::support::{
    BASE, KEY, NO_CHUNKS, STORE, block_on, body_json, capture, created_store, defaults, finalized,
    fixtures, gemini_only, header, json, keyless, options, store, upload_url, with_chunking,
    with_store_id,
};
use self::transport::{ScriptedReply, reply};
use super::port::{TransportBody, TransportMethod, TransportRequest};
use super::{EmbedStep as _, GeminiDefaults, GeminiIngestion, GeminiIngestionError};
use crate::rag::ingestion::IngestionState;
use crate::rag::ingestion::store::{RagStore as _, StoreRequest};

#[test]
fn flags_supports_existing_file_id_is_false() {
    assert!(!GeminiIngestion::SUPPORTS_EXISTING_FILE_ID);
    assert!(!IngestionState::SUPPORTS_EXISTING_FILE_ID);
}

#[test]
fn flags_embed_returns_no_embeddings() {
    let fixture = fixtures(options(None, gemini_only()), defaults(), Vec::new());
    let chunks = vec!["one".to_owned()];
    assert_eq!(fixture.ingestion.embed(&chunks), None);
}

#[tokio::test]
async fn credentials_missing_key_fails_before_any_request() {
    let fixture = fixtures(options(None, gemini_only()), keyless(), Vec::new());
    let error = store(&fixture, None, None, None).await.unwrap_err();
    assert!(matches!(error, GeminiIngestionError::MissingApiKey));
    assert!(fixture.transport.take_sent().is_empty());
}

#[tokio::test]
async fn credentials_config_key_wins_over_the_default() {
    let fixture = fixtures(
        options(
            Some("notes"),
            r#"{"custom_llm_provider":"gemini","api_key":"config-key"}"#,
        ),
        defaults(),
        vec![created_store()],
    );
    store(&fixture, None, None, None).await.unwrap();
    let sent = fixture.transport.take_sent();
    assert_eq!(header(&sent[0].headers, "x-goog-api-key"), "config-key");
}

#[tokio::test]
async fn credentials_empty_config_key_falls_back_to_the_default() {
    let fixture = fixtures(
        options(
            Some("notes"),
            r#"{"custom_llm_provider":"gemini","api_key":""}"#,
        ),
        defaults(),
        vec![created_store()],
    );
    store(&fixture, None, None, None).await.unwrap();
    let sent = fixture.transport.take_sent();
    assert_eq!(header(&sent[0].headers, "x-goog-api-key"), KEY);
}

#[tokio::test]
async fn credentials_empty_default_key_counts_as_missing() {
    let empty = GeminiDefaults {
        api_key: Some(SecretString::from(String::new())),
        api_base: BASE.to_owned(),
    };
    let fixture = fixtures(options(None, gemini_only()), empty, Vec::new());
    let error = store(&fixture, None, None, None).await.unwrap_err();
    assert!(matches!(error, GeminiIngestionError::MissingApiKey));
    assert!(fixture.transport.take_sent().is_empty());
}

#[tokio::test]
async fn credentials_config_base_wins_over_the_default() {
    let fixture = fixtures(
        options(
            Some("notes"),
            r#"{"custom_llm_provider":"gemini","api_base":"https://config.test"}"#,
        ),
        defaults(),
        vec![created_store()],
    );
    store(&fixture, None, None, None).await.unwrap();
    let sent = fixture.transport.take_sent();
    assert_eq!(sent[0].url, "https://config.test/v1beta/fileSearchStores");
}

#[tokio::test]
async fn credentials_empty_config_base_falls_back_to_the_default() {
    let fixture = fixtures(
        options(
            Some("notes"),
            r#"{"custom_llm_provider":"gemini","api_base":""}"#,
        ),
        defaults(),
        vec![created_store()],
    );
    store(&fixture, None, None, None).await.unwrap();
    let sent = fixture.transport.take_sent();
    assert_eq!(sent[0].url, "https://gemini.test/v1beta/fileSearchStores");
}

#[tokio::test]
async fn credentials_null_config_key_falls_back_to_the_default() {
    let fixture = fixtures(
        options(
            Some("notes"),
            r#"{"custom_llm_provider":"gemini","api_key":null}"#,
        ),
        defaults(),
        vec![created_store()],
    );
    store(&fixture, None, None, None).await.unwrap();
    let sent = fixture.transport.take_sent();
    assert_eq!(header(&sent[0].headers, "x-goog-api-key"), KEY);
}

#[tokio::test]
async fn credentials_null_config_base_falls_back_to_the_default() {
    let fixture = fixtures(
        options(
            Some("notes"),
            r#"{"custom_llm_provider":"gemini","api_base":null}"#,
        ),
        defaults(),
        vec![created_store()],
    );
    store(&fixture, None, None, None).await.unwrap();
    let sent = fixture.transport.take_sent();
    assert_eq!(sent[0].url, "https://gemini.test/v1beta/fileSearchStores");
}

#[test]
fn credentials_request_debug_shows_header_names_only() {
    let mut headers = HeaderMap::new();
    headers.insert("x-goog-api-key", HeaderValue::from_static(KEY));
    let request = TransportRequest {
        method: TransportMethod::Post,
        url: "https://gemini.test/v1beta/fileSearchStores".to_owned(),
        headers,
        body: TransportBody::Bytes(b"hello".to_vec()),
        timeout: Duration::from_secs(60),
    };
    let shown = format!("{request:?}");
    assert!(!shown.contains(KEY));
    assert!(shown.contains("x-goog-api-key"));
    assert!(shown.contains("body_bytes: Some(5)"));
}

#[tokio::test]
async fn credentials_key_never_reaches_debug_or_error_text() {
    let configured = defaults();
    assert!(!format!("{configured:?}").contains(KEY));
    let fixture = fixtures(
        options(Some("notes"), gemini_only()),
        configured,
        vec![reply(500, &[], b"boom")],
    );
    let error = store(&fixture, None, None, None).await.unwrap_err();
    assert!(!error.to_string().contains(KEY));
    assert!(
        error
            .to_string()
            .contains("Failed to create File Search store: boom")
    );
    let sent = fixture.transport.take_sent();
    assert_eq!(header(&sent[0].headers, "x-goog-api-key"), KEY);
}

#[tokio::test]
async fn credentials_wrong_type_names_the_field() {
    let fixture = fixtures(
        options(
            Some("notes"),
            r#"{"custom_llm_provider":"gemini","api_key":5}"#,
        ),
        defaults(),
        Vec::new(),
    );
    let error = store(&fixture, None, None, None).await.unwrap_err();
    assert!(error.to_string().contains("vector_store.api_key"));
    assert!(error.to_string().contains("must be a string"));
}

#[tokio::test]
async fn create_store_posts_the_display_name_with_the_key() {
    let fixture = fixtures(
        options(Some("my-store"), gemini_only()),
        defaults(),
        vec![created_store()],
    );
    let result = store(&fixture, None, None, None).await.unwrap();
    assert_eq!(result, (Some(STORE.to_owned()), None));
    let sent = fixture.transport.take_sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "POST");
    assert_eq!(sent[0].url, "https://gemini.test/v1beta/fileSearchStores");
    assert_eq!(sent[0].timeout, Duration::from_secs(60));
    assert_eq!(header(&sent[0].headers, "content-type"), "application/json");
    assert_eq!(header(&sent[0].headers, "x-goog-api-key"), KEY);
    assert_eq!(body_json(&sent[0]), json(r#"{"displayName":"my-store"}"#));
}

#[rstest::rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[tokio::test]
async fn create_store_uses_the_default_ingest_name(#[case] name: Option<&str>) {
    let fixture = fixtures(
        options(name, gemini_only()),
        defaults(),
        vec![created_store()],
    );
    store(&fixture, None, None, None).await.unwrap();
    let sent = fixture.transport.take_sent();
    assert_eq!(
        body_json(&sent[0]),
        json(r#"{"displayName":"litellm-rag-ingest"}"#)
    );
}

#[tokio::test]
async fn create_store_without_a_name_leaves_an_empty_store_id() {
    let fixture = fixtures(
        options(Some("notes"), gemini_only()),
        defaults(),
        vec![reply(200, &[], b"{}")],
    );
    let result = store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap();
    assert_eq!(result, (Some(String::new()), None));
    assert_eq!(fixture.transport.take_sent().len(), 1);
}

#[rstest::rstest]
#[case::empty("\"\"")]
#[case::null("null")]
#[tokio::test]
async fn create_store_when_the_config_store_id_is_empty_or_null(#[case] store_id: &str) {
    let fixture = fixtures(
        options(
            Some("notes"),
            &format!(r#"{{"custom_llm_provider":"gemini","vector_store_id":{store_id}}}"#),
        ),
        defaults(),
        vec![created_store()],
    );
    let result = store(&fixture, None, None, None).await.unwrap();
    assert_eq!(result, (Some(STORE.to_owned()), None));
    let sent = fixture.transport.take_sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].url, "https://gemini.test/v1beta/fileSearchStores");
}

#[rstest::rstest]
#[case::created(201)]
#[case::bad_request(400)]
#[case::server_error(500)]
#[tokio::test]
async fn create_store_rejects_any_status_but_200(#[case] status: u16) {
    let fixture = fixtures(
        options(Some("notes"), gemini_only()),
        defaults(),
        vec![reply(status, &[], br#"{"name":"fileSearchStores/s"}"#)],
    );
    let error = store(&fixture, None, None, None).await.unwrap_err();
    assert!(matches!(
        error,
        GeminiIngestionError::CreateStoreFailed { .. }
    ));
    assert!(
        error
            .to_string()
            .contains(r#"Failed to create File Search store: {"name":"fileSearchStores/s"}"#)
    );
}

#[tokio::test]
async fn create_store_rejects_an_unparsable_reply() {
    let fixture = fixtures(
        options(Some("notes"), gemini_only()),
        defaults(),
        vec![reply(200, &[], b"not json")],
    );
    let error = store(&fixture, None, None, None).await.unwrap_err();
    assert!(matches!(error, GeminiIngestionError::BadReply { .. }));
}

#[tokio::test]
async fn create_store_rejects_a_non_object_reply() {
    let fixture = fixtures(
        options(Some("notes"), gemini_only()),
        defaults(),
        vec![reply(200, &[], b"[]")],
    );
    let error = store(&fixture, None, None, None).await.unwrap_err();
    assert!(matches!(error, GeminiIngestionError::BadReply { .. }));
}

#[test]
fn create_store_logs_the_failure_at_error_level() {
    let fixture = fixtures(
        options(Some("notes"), gemini_only()),
        defaults(),
        vec![reply(500, &[], b"boom")],
    );
    let (result, logs) = capture(|| block_on(store(&fixture, None, None, None)));
    assert!(result.is_err());
    assert!(logs.iter().any(|(level, message)| *level == Level::ERROR
        && message == "Failed to create File Search store: boom"));
}

#[tokio::test]
async fn initiate_builds_the_upload_url_and_resumable_headers() {
    let fixture = fixtures(
        options(None, with_store_id()),
        defaults(),
        vec![upload_url(), finalized("files/1")],
    );
    let result = store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap();
    assert_eq!(result, (Some(STORE.to_owned()), Some("files/1".to_owned())));
    let sent = fixture.transport.take_sent();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].method, "POST");
    assert_eq!(
        sent[0].url,
        "https://gemini.test/upload/v1beta/fileSearchStores/s:uploadToFileSearchStore"
    );
    assert_eq!(sent[0].timeout, Duration::from_secs(60));
    assert_eq!(
        header(&sent[0].headers, "x-goog-upload-protocol"),
        "resumable"
    );
    assert_eq!(header(&sent[0].headers, "x-goog-upload-command"), "start");
    assert_eq!(
        header(&sent[0].headers, "x-goog-upload-header-content-length"),
        "5"
    );
    assert_eq!(
        header(&sent[0].headers, "x-goog-upload-header-content-type"),
        "application/octet-stream"
    );
    assert_eq!(header(&sent[0].headers, "content-type"), "application/json");
    assert_eq!(header(&sent[0].headers, "x-goog-api-key"), KEY);
    assert_eq!(body_json(&sent[0]), json(r#"{"displayName":"a.txt"}"#));
}

#[tokio::test]
async fn initiate_uses_the_given_content_type() {
    let fixture = fixtures(
        options(None, with_store_id()),
        defaults(),
        vec![upload_url(), finalized("files/1")],
    );
    store(&fixture, Some(b"hello"), Some("a.txt"), Some("text/plain"))
        .await
        .unwrap();
    let sent = fixture.transport.take_sent();
    assert_eq!(
        header(&sent[0].headers, "x-goog-upload-header-content-type"),
        "text/plain"
    );
}

#[tokio::test]
async fn initiate_defaults_an_empty_content_type() {
    let fixture = fixtures(
        options(None, with_store_id()),
        defaults(),
        vec![upload_url(), finalized("files/1")],
    );
    store(&fixture, Some(b"hello"), Some("a.txt"), Some(""))
        .await
        .unwrap();
    let sent = fixture.transport.take_sent();
    assert_eq!(
        header(&sent[0].headers, "x-goog-upload-header-content-type"),
        "application/octet-stream"
    );
}

#[tokio::test]
async fn initiate_strips_every_version_segment() {
    let fixture = fixtures(
        options(
            None,
            r#"{"custom_llm_provider":"gemini","api_base":"https://gemini.test/v1beta/proxy","vector_store_id":"fileSearchStores/s"}"#,
        ),
        defaults(),
        vec![upload_url(), finalized("files/1")],
    );
    store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap();
    let sent = fixture.transport.take_sent();
    assert_eq!(
        sent[0].url,
        "https://gemini.test/proxy/upload/v1beta/fileSearchStores/s:uploadToFileSearchStore"
    );
}

#[tokio::test]
async fn initiate_adds_the_white_space_config() {
    let chunked = with_chunking(
        options(None, with_store_id()),
        r#"{"white_space_config":{"max_tokens_per_chunk":0}}"#,
    );
    let fixture = fixtures(
        chunked,
        defaults(),
        vec![upload_url(), finalized("files/1")],
    );
    store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap();
    let sent = fixture.transport.take_sent();
    assert_eq!(
        body_json(&sent[0]),
        json(
            r#"{"displayName":"a.txt","chunkingConfig":{"whiteSpaceConfig":{"maxTokensPerChunk":0,"maxOverlapTokens":400}}}"#
        )
    );
}

#[tokio::test]
async fn initiate_defaults_the_token_limits() {
    let chunked = with_chunking(
        options(None, with_store_id()),
        r#"{"white_space_config":{"max_overlap_tokens":7}}"#,
    );
    let fixture = fixtures(
        chunked,
        defaults(),
        vec![upload_url(), finalized("files/1")],
    );
    store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap();
    let sent = fixture.transport.take_sent();
    assert_eq!(
        body_json(&sent[0]),
        json(
            r#"{"displayName":"a.txt","chunkingConfig":{"whiteSpaceConfig":{"maxTokensPerChunk":800,"maxOverlapTokens":7}}}"#
        )
    );
}

#[tokio::test]
async fn initiate_passes_through_present_null_token_limits() {
    let chunked = with_chunking(
        options(None, with_store_id()),
        r#"{"white_space_config":{"max_tokens_per_chunk":null,"max_overlap_tokens":null}}"#,
    );
    let fixture = fixtures(
        chunked,
        defaults(),
        vec![upload_url(), finalized("files/1")],
    );
    store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap();
    let sent = fixture.transport.take_sent();
    assert_eq!(
        body_json(&sent[0]),
        json(
            r#"{"displayName":"a.txt","chunkingConfig":{"whiteSpaceConfig":{"maxTokensPerChunk":null,"maxOverlapTokens":null}}}"#
        )
    );
}

#[tokio::test]
async fn initiate_skips_an_empty_white_space_config() {
    let chunked = with_chunking(
        options(None, with_store_id()),
        r#"{"white_space_config":{}}"#,
    );
    let fixture = fixtures(
        chunked,
        defaults(),
        vec![upload_url(), finalized("files/1")],
    );
    store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap();
    let sent = fixture.transport.take_sent();
    assert_eq!(body_json(&sent[0]), json(r#"{"displayName":"a.txt"}"#));
}

#[tokio::test]
async fn initiate_adds_custom_metadata() {
    let fixture = fixtures(
        options(
            None,
            r#"{"custom_llm_provider":"gemini","vector_store_id":"fileSearchStores/s","custom_metadata":[{"key":"k","stringValue":"v"}]}"#,
        ),
        defaults(),
        vec![upload_url(), finalized("files/1")],
    );
    store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap();
    let sent = fixture.transport.take_sent();
    assert_eq!(
        body_json(&sent[0]),
        json(r#"{"displayName":"a.txt","customMetadata":[{"key":"k","stringValue":"v"}]}"#)
    );
}

#[rstest::rstest]
#[case::empty_list("[]")]
#[case::null("null")]
#[tokio::test]
async fn initiate_skips_an_empty_custom_metadata(#[case] metadata: &str) {
    let fixture = fixtures(
        options(
            None,
            &format!(
                r#"{{"custom_llm_provider":"gemini","vector_store_id":"fileSearchStores/s","custom_metadata":{metadata}}}"#
            ),
        ),
        defaults(),
        vec![upload_url(), finalized("files/1")],
    );
    store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap();
    let sent = fixture.transport.take_sent();
    assert_eq!(body_json(&sent[0]), json(r#"{"displayName":"a.txt"}"#));
}

#[tokio::test]
async fn initiate_requires_the_upload_url_header() {
    let fixture = fixtures(
        options(None, with_store_id()),
        defaults(),
        vec![reply(200, &[], b"{}")],
    );
    let error = store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap_err();
    assert!(matches!(error, GeminiIngestionError::NoUploadUrl));
}

#[tokio::test]
async fn initiate_rejects_an_empty_upload_url() {
    let fixture = fixtures(
        options(None, with_store_id()),
        defaults(),
        vec![reply(200, &[("x-goog-upload-url", "")], b"{}")],
    );
    let error = store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap_err();
    assert!(matches!(error, GeminiIngestionError::NoUploadUrl));
}

#[tokio::test]
async fn initiate_accepts_created() {
    let fixture = fixtures(
        options(None, with_store_id()),
        defaults(),
        vec![
            reply(
                201,
                &[("x-goog-upload-url", "https://upload.test/session")],
                b"{}",
            ),
            finalized("files/1"),
        ],
    );
    let result = store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap();
    assert_eq!(result.1, Some("files/1".to_owned()));
}

#[tokio::test]
async fn initiate_rejects_other_statuses() {
    let fixture = fixtures(
        options(None, with_store_id()),
        defaults(),
        vec![reply(500, &[], b"boom")],
    );
    let error = store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap_err();
    assert!(matches!(error, GeminiIngestionError::InitiateFailed { .. }));
    assert!(
        error
            .to_string()
            .contains("Failed to initiate upload: boom")
    );
}

#[tokio::test]
async fn finalize_puts_the_bytes_without_the_api_key() {
    let fixture = fixtures(
        options(None, with_store_id()),
        defaults(),
        vec![upload_url(), finalized("files/1")],
    );
    store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap();
    let sent = fixture.transport.take_sent();
    assert_eq!(sent[1].method, "PUT");
    assert_eq!(sent[1].url, "https://upload.test/session");
    assert_eq!(sent[1].timeout, Duration::from_secs(300));
    assert_eq!(sent[1].body, b"hello");
    assert_eq!(sent[1].headers.len(), 3);
    assert_eq!(header(&sent[1].headers, "content-length"), "5");
    assert_eq!(header(&sent[1].headers, "x-goog-upload-offset"), "0");
    assert_eq!(
        header(&sent[1].headers, "x-goog-upload-command"),
        "upload, finalize"
    );
    assert!(sent[1].headers.get("x-goog-api-key").is_none());
}

#[tokio::test]
async fn finalize_reads_a_top_level_name() {
    let fixture = fixtures(
        options(None, with_store_id()),
        defaults(),
        vec![upload_url(), finalized("files/top")],
    );
    let result = store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap();
    assert_eq!(result.1, Some("files/top".to_owned()));
}

#[tokio::test]
async fn finalize_reads_a_nested_file_name() {
    let fixture = fixtures(
        options(None, with_store_id()),
        defaults(),
        vec![
            upload_url(),
            reply(200, &[], br#"{"name":"","file":{"name":"files/nested"}}"#),
        ],
    );
    let result = store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap();
    assert_eq!(result.1, Some("files/nested".to_owned()));
}

#[tokio::test]
async fn finalize_returns_an_empty_id_when_the_name_is_absent() {
    let fixture = fixtures(
        options(None, with_store_id()),
        defaults(),
        vec![upload_url(), reply(200, &[], b"{}")],
    );
    let result = store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap();
    assert_eq!(result.1, Some(String::new()));
}

#[test]
fn finalize_uses_the_placeholder_for_an_unreadable_reply() {
    let fixture = fixtures(
        options(None, with_store_id()),
        defaults(),
        vec![upload_url(), reply(200, &[], b"not json")],
    );
    let (result, logs) = capture(|| block_on(store(&fixture, Some(b"hello"), Some("a.txt"), None)));
    assert_eq!(result.unwrap().1, Some("uploaded".to_owned()));
    assert!(logs.iter().any(|(level, message)| *level == Level::WARN
        && message.starts_with("Could not parse upload response")));
}

#[tokio::test]
async fn finalize_uses_the_placeholder_for_a_null_file() {
    let fixture = fixtures(
        options(None, with_store_id()),
        defaults(),
        vec![upload_url(), reply(200, &[], br#"{"name":"","file":null}"#)],
    );
    let result = store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap();
    assert_eq!(result.1, Some("uploaded".to_owned()));
}

#[test]
fn finalize_uses_the_placeholder_for_a_non_object_reply() {
    let fixture = fixtures(
        options(None, with_store_id()),
        defaults(),
        vec![upload_url(), reply(200, &[], b"[]")],
    );
    let (result, logs) = capture(|| block_on(store(&fixture, Some(b"hello"), Some("a.txt"), None)));
    assert_eq!(result.unwrap().1, Some("uploaded".to_owned()));
    assert!(logs.iter().any(|(level, message)| *level == Level::WARN
        && message.starts_with("Could not parse upload response")));
}

#[tokio::test]
async fn finalize_accepts_created() {
    let fixture = fixtures(
        options(None, with_store_id()),
        defaults(),
        vec![
            upload_url(),
            reply(201, &[], br#"{"name":"files/created"}"#),
        ],
    );
    let result = store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap();
    assert_eq!(result.1, Some("files/created".to_owned()));
}

#[tokio::test]
async fn finalize_rejects_other_statuses() {
    let fixture = fixtures(
        options(None, with_store_id()),
        defaults(),
        vec![upload_url(), reply(500, &[], b"boom")],
    );
    let error = store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap_err();
    assert!(matches!(error, GeminiIngestionError::UploadFailed { .. }));
    assert!(error.to_string().contains("Failed to upload file: boom"));
}

#[tokio::test]
async fn store_flow_skips_the_upload_for_empty_file_bytes() {
    let fixture = fixtures(options(None, with_store_id()), defaults(), Vec::new());
    let result = store(&fixture, Some(b""), Some("a.txt"), None)
        .await
        .unwrap();
    assert_eq!(result, (Some(STORE.to_owned()), None));
    assert!(fixture.transport.take_sent().is_empty());
}

#[tokio::test]
async fn store_flow_skips_the_upload_for_an_empty_filename() {
    let fixture = fixtures(options(None, with_store_id()), defaults(), Vec::new());
    let result = store(&fixture, Some(b"hello"), Some(""), None)
        .await
        .unwrap();
    assert_eq!(result, (Some(STORE.to_owned()), None));
    assert!(fixture.transport.take_sent().is_empty());
}

#[tokio::test]
async fn store_flow_skips_the_upload_for_an_empty_store_id() {
    let fixture = fixtures(
        options(Some("notes"), gemini_only()),
        defaults(),
        vec![reply(200, &[], b"{}")],
    );
    let result = store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap();
    assert_eq!(result, (Some(String::new()), None));
    assert_eq!(fixture.transport.take_sent().len(), 1);
}

#[tokio::test]
async fn store_flow_keeps_the_created_store_when_the_upload_fails() {
    let fixture = fixtures(
        options(Some("notes"), gemini_only()),
        defaults(),
        vec![created_store(), reply(500, &[], b"boom")],
    );
    let error = store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap_err();
    assert!(matches!(error, GeminiIngestionError::InitiateFailed { .. }));
    let sent = fixture.transport.take_sent();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].url, "https://gemini.test/v1beta/fileSearchStores");
}

#[tokio::test]
async fn store_flow_reports_a_transport_failure() {
    let fixture = fixtures(
        options(None, with_store_id()),
        defaults(),
        vec![ScriptedReply::Failure("the socket closed".to_owned())],
    );
    let error = store(&fixture, Some(b"hello"), Some("a.txt"), None)
        .await
        .unwrap_err();
    assert!(matches!(error, GeminiIngestionError::Transport(_)));
    assert!(error.to_string().contains("the socket closed"));
    let sent = fixture.transport.take_sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, "POST");
}

#[tokio::test]
async fn store_flow_ignores_the_existing_file_id_argument() {
    let fixture = fixtures(
        options(None, with_store_id()),
        defaults(),
        vec![upload_url(), finalized("files/1")],
    );
    let request = StoreRequest {
        file_content: Some(b"hello"),
        filename: Some("a.txt"),
        content_type: None,
        chunks: NO_CHUNKS,
        embeddings: None,
        existing_file_id: Some("existing-file"),
    };
    let result = fixture.ingestion.store(request).await.unwrap();
    assert_eq!(result.1, Some("files/1".to_owned()));
}
