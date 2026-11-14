use std::time::Duration;

use http::Method;
use rstest::rstest;

use super::super::support::{Fixture, MODEL, Script, Sent, json, run, secret};
use crate::endpoint_target::error::EndpointTargetError;
use crate::endpoint_target::target::TargetProvider;
use crate::gemini_embedding_call::call::Execution;
use crate::gemini_embedding_call::error::GeminiEmbeddingCallError;
use crate::gemini_embedding_call::execute::Clients;

const VERTEX_REPLY: &str = r#"{"embedding":{"values":[0.5]}}"#;
const STUDIO_REPLY: &str = r#"{"embeddings":[{"values":[0.5]}]}"#;
const STUDIO_POST: &str = "https://generativelanguage.googleapis.com/v1beta/models/gemini-embedding-001:batchEmbedContents";
const PDF: &str = r#"{"mimeType":"application/pdf","uri":"https://generativelanguage.googleapis.com/v1beta/files/a"}"#;

fn pooled_only(pooled: &Script) -> Clients<'_, Script> {
    Clients {
        supplied: None,
        pooled,
    }
}

fn header<'sent>(sent: &'sent Sent, name: &str) -> Option<&'sent str> {
    sent.headers.get(name).map(|value| value.to_str().unwrap())
}

#[tokio::test]
async fn vertex_posts_embed_content_with_the_minted_token() {
    let fixture = Fixture::new(r#""hello""#);
    let pooled = Script::answering(&[(200, VERTEX_REPLY)]);
    let reply = run(
        &fixture.call(TargetProvider::Vertex, Execution::Sync),
        pooled_only(&pooled),
    )
    .await
    .unwrap();
    assert_eq!(reply.model, MODEL);
    assert_eq!(reply.data.len(), 1);
    let sent = pooled.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, Method::POST);
    assert_eq!(
        sent[0].url,
        "https://us-central1-aiplatform.googleapis.com/v1/projects/proj/locations/us-central1/publishers/google/models/gemini-embedding-001:embedContent"
    );
    assert_eq!(header(&sent[0], "authorization"), Some("Bearer minted"));
    assert_eq!(
        header(&sent[0], "content-type"),
        Some("application/json; charset=utf-8")
    );
    assert_eq!(
        json(std::str::from_utf8(&sent[0].body).unwrap()),
        json(r#"{"content":{"parts":[{"text":"hello"}]}}"#)
    );
}

#[tokio::test]
async fn studio_posts_batch_embed_contents_with_the_key_header() {
    let mut fixture = Fixture::new(r#"["a","b"]"#);
    fixture.api_key = Some(secret("gk"));
    let pooled = Script::answering(&[(
        200,
        r#"{"embeddings":[{"values":[0.5]},{"values":[0.25]}]}"#,
    )]);
    let reply = run(
        &fixture.call(TargetProvider::Gemini, Execution::Sync),
        pooled_only(&pooled),
    )
    .await
    .unwrap();
    assert_eq!(reply.data.len(), 2);
    let sent = pooled.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].url, STUDIO_POST);
    assert_eq!(header(&sent[0], "x-goog-api-key"), Some("gk"));
    assert_eq!(header(&sent[0], "authorization"), None);
    assert!(sent[0].headers["x-goog-api-key"].is_sensitive());
    assert_eq!(
        json(std::str::from_utf8(&sent[0].body).unwrap()),
        json(
            r#"{"requests":[
                {"model":"models/gemini-embedding-001","content":{"parts":[{"text":"a"}]}},
                {"model":"models/gemini-embedding-001","content":{"parts":[{"text":"b"}]}}
            ]}"#
        )
    );
}

#[tokio::test]
async fn studio_fetches_every_file_occurrence_then_posts_the_metadata() {
    let mut fixture = Fixture::new(r#"["files/a",["files/b","files/a"]]"#);
    fixture.api_key = Some(secret("gk"));
    let other = r#"{"mimeType":"image/png","uri":"https://generativelanguage.googleapis.com/v1beta/files/b"}"#;
    let later = r#"{"mimeType":"text/plain","uri":"https://generativelanguage.googleapis.com/v1beta/files/a2"}"#;
    let pooled = Script::answering(&[
        (200, PDF),
        (200, other),
        (200, later),
        (200, r#"{"embeddings":[{"values":[0.5]},{"values":[0.5]}]}"#),
    ]);
    run(
        &fixture.call(TargetProvider::Gemini, Execution::Sync),
        pooled_only(&pooled),
    )
    .await
    .unwrap();
    let sent = pooled.sent();
    let urls: Vec<&str> = sent.iter().map(|each| each.url.as_str()).collect();
    assert_eq!(
        urls,
        [
            "https://generativelanguage.googleapis.com/v1beta/files/a",
            "https://generativelanguage.googleapis.com/v1beta/files/b",
            "https://generativelanguage.googleapis.com/v1beta/files/a",
            STUDIO_POST,
        ]
    );
    assert!(sent[..3].iter().all(|each| each.method == Method::GET));
    assert!(sent[..3].iter().all(|each| each.body.is_empty()));
    assert_eq!(header(&sent[0], "x-goog-api-key"), Some("gk"));
    assert_eq!(header(&sent[0], "authorization"), None);
    assert_eq!(header(&sent[0], "content-type"), None);
    let body = std::str::from_utf8(&sent[3].body).unwrap();
    assert!(
        body.contains(r#""file_uri":"https://generativelanguage.googleapis.com/v1beta/files/a2""#)
    );
    assert!(body.contains(r#""mime_type":"image/png""#));
    assert!(body.contains(r#""mime_type":"text/plain""#));
    assert!(
        !body.contains(r#""file_uri":"https://generativelanguage.googleapis.com/v1beta/files/a""#)
    );
}

#[tokio::test]
async fn a_custom_base_moves_the_post_but_not_the_file_lookups() {
    let mut fixture = Fixture::new(r#""files/a""#);
    fixture.api_key = Some(secret("gk"));
    fixture.api_base = Some(String::from("https://custom.example.com"));
    let pooled = Script::answering(&[(200, PDF), (200, STUDIO_REPLY)]);
    run(
        &fixture.call(TargetProvider::Gemini, Execution::Sync),
        pooled_only(&pooled),
    )
    .await
    .unwrap();
    let sent = pooled.sent();
    assert_eq!(
        sent[0].url,
        "https://generativelanguage.googleapis.com/v1beta/files/a"
    );
    assert_eq!(
        sent[1].url,
        "https://custom.example.com/models/gemini-embedding-001:batchEmbedContents"
    );
}

#[tokio::test]
async fn vertex_with_a_key_resolves_top_level_references_only() {
    let mut fixture = Fixture::new(r#"["files/a","gs://bucket/b.pdf"]"#);
    fixture.api_key = Some(secret("gk"));
    let pooled = Script::answering(&[(200, PDF), (200, VERTEX_REPLY)]);
    run(
        &fixture.call(TargetProvider::Vertex, Execution::Sync),
        pooled_only(&pooled),
    )
    .await
    .unwrap();
    let sent = pooled.sent();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].method, Method::GET);
    assert_eq!(
        sent[0].url,
        "https://generativelanguage.googleapis.com/v1beta/files/a"
    );
    assert_eq!(sent[1].method, Method::POST);
    let body = std::str::from_utf8(&sent[1].body).unwrap();
    assert!(body.contains(r#""file_uri":"gs://bucket/b.pdf""#));
}

#[tokio::test]
async fn vertex_without_a_key_downloads_nothing() {
    let fixture = Fixture::new(r#""files/a""#);
    let pooled = Script::answering(&[(200, VERTEX_REPLY)]);
    run(
        &fixture.call(TargetProvider::Vertex, Execution::Sync),
        pooled_only(&pooled),
    )
    .await
    .unwrap();
    let sent = pooled.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, Method::POST);
}

#[tokio::test]
async fn gcs_references_are_forwarded_without_a_download() {
    let mut fixture = Fixture::new(r#"["gs://bucket/a.pdf"]"#);
    fixture.api_key = Some(secret("gk"));
    let pooled = Script::answering(&[(200, STUDIO_REPLY)]);
    run(
        &fixture.call(TargetProvider::Gemini, Execution::Sync),
        pooled_only(&pooled),
    )
    .await
    .unwrap();
    assert_eq!(pooled.sent().len(), 1);
}

#[tokio::test]
async fn caller_headers_beat_every_default() {
    let mut fixture = Fixture::new(r#""hello""#);
    fixture.api_key = Some(secret("gk"));
    fixture.extra_headers = [
        ("X-Goog-Api-Key", "override"),
        ("content-type", "text/plain"),
        ("X-Extra", "1"),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value.to_owned()))
    .collect();
    let pooled = Script::answering(&[(200, STUDIO_REPLY)]);
    run(
        &fixture.call(TargetProvider::Gemini, Execution::Sync),
        pooled_only(&pooled),
    )
    .await
    .unwrap();
    let sent = pooled.sent();
    assert_eq!(header(&sent[0], "x-goog-api-key"), Some("override"));
    assert_eq!(header(&sent[0], "content-type"), Some("text/plain"));
    assert_eq!(header(&sent[0], "x-extra"), Some("1"));
}

#[rstest]
#[case::sync_supplied(Execution::Sync, true, None, 600)]
#[case::sync_supplied_ignores_the_timeout(Execution::Sync, true, Some(10), 600)]
#[case::sync_created(Execution::Sync, false, None, 600)]
#[case::sync_created_with_a_timeout(Execution::Sync, false, Some(10), 10)]
#[case::async_without_a_timeout(Execution::Async, true, None, 600)]
#[case::async_with_a_timeout(Execution::Async, true, Some(10), 10)]
#[tokio::test]
async fn the_deadline_follows_the_execution(
    #[case] execution: Execution,
    #[case] supplied: bool,
    #[case] seconds: Option<u64>,
    #[case] deadline: u64,
) {
    let mut fixture = Fixture::new(r#""hello""#);
    fixture.timeout = seconds.map(Duration::from_secs);
    let pooled = Script::answering(&[(200, VERTEX_REPLY)]);
    let supplied_script = Script::answering(&[(200, VERTEX_REPLY)]);
    run(
        &fixture.call(TargetProvider::Vertex, execution),
        Clients {
            supplied: supplied.then_some(&supplied_script),
            pooled: &pooled,
        },
    )
    .await
    .unwrap();
    let used = if execution == Execution::Sync && supplied {
        &supplied_script
    } else {
        &pooled
    };
    assert_eq!(used.sent().len(), 1);
    assert_eq!(used.sent()[0].deadline, Duration::from_secs(deadline));
    let unused = if execution == Execution::Sync && supplied {
        &pooled
    } else {
        &supplied_script
    };
    assert!(unused.sent().is_empty());
}

#[tokio::test]
async fn studio_without_a_key_fails_before_any_request() {
    let fixture = Fixture::new(r#""files/a""#);
    let pooled = Script::default();
    let error = run(
        &fixture.call(TargetProvider::Gemini, Execution::Sync),
        pooled_only(&pooled),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        GeminiEmbeddingCallError::Target(EndpointTargetError::GeminiApiKey)
    ));
    assert!(pooled.sent().is_empty());
}

#[rstest]
#[case::not_found(404)]
#[case::created(201)]
#[case::server_error(500)]
#[tokio::test]
async fn a_file_lookup_must_answer_200(#[case] status: u16) {
    let mut fixture = Fixture::new(r#"["files/a","files/b"]"#);
    fixture.api_key = Some(secret("gk"));
    let pooled = Script::answering(&[(status, "denied")]);
    let error = run(
        &fixture.call(TargetProvider::Gemini, Execution::Sync),
        pooled_only(&pooled),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        &error,
        GeminiEmbeddingCallError::FileStatus { file, status: seen, body }
            if file == "files/a" && *seen == status && body == "denied"
    ));
    assert_eq!(
        error.to_string(),
        format!("Error fetching file files/a: {status} denied")
    );
    assert_eq!(pooled.sent().len(), 1);
}

#[tokio::test]
async fn a_failed_lookup_replaces_invalid_utf8_in_its_message() {
    let mut fixture = Fixture::new(r#""files/a""#);
    fixture.api_key = Some(secret("gk"));
    let pooled = Script::answering_bytes(&[(500, b"bad \xff body")]);
    let error = run(
        &fixture.call(TargetProvider::Gemini, Execution::Sync),
        pooled_only(&pooled),
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Error fetching file files/a: 500 bad \u{FFFD} body"
    );
}

#[rstest]
#[case::bad_request(400)]
#[case::created(201)]
#[case::no_content(204)]
#[tokio::test]
async fn an_embedding_post_must_answer_200(#[case] status: u16) {
    let fixture = Fixture::new(r#""hello""#);
    let pooled = Script::answering(&[(status, "nope")]);
    let error = run(
        &fixture.call(TargetProvider::Vertex, Execution::Sync),
        pooled_only(&pooled),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        &error,
        GeminiEmbeddingCallError::PostStatus { status: seen, body } if *seen == status && body == "nope"
    ));
    assert_eq!(error.to_string(), format!("Error: {status} nope"));
}

#[tokio::test]
async fn a_failed_connection_names_the_url() {
    let fixture = Fixture::new(r#""hello""#);
    let pooled = Script::default();
    let error = run(
        &fixture.call(TargetProvider::Vertex, Execution::Sync),
        pooled_only(&pooled),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        &error,
        GeminiEmbeddingCallError::Transport { url, .. } if url.ends_with(":embedContent")
    ));
}

#[tokio::test]
async fn a_reply_that_is_not_json_is_a_decode_error() {
    let fixture = Fixture::new(r#""hello""#);
    let pooled = Script::answering(&[(200, "<html>")]);
    let error = run(
        &fixture.call(TargetProvider::Vertex, Execution::Sync),
        pooled_only(&pooled),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, GeminiEmbeddingCallError::Decode(_)));
}

#[tokio::test]
async fn a_reply_without_embeddings_is_a_records_error() {
    let fixture = Fixture::new(r#""hello""#);
    let pooled = Script::answering(&[(200, "{}")]);
    let error = run(
        &fixture.call(TargetProvider::Vertex, Execution::Sync),
        pooled_only(&pooled),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, GeminiEmbeddingCallError::Records(_)));
}

#[tokio::test]
async fn a_file_reply_that_is_not_an_object_fails_the_call() {
    let mut fixture = Fixture::new(r#""files/a""#);
    fixture.api_key = Some(secret("gk"));
    let pooled = Script::answering(&[(200, "[]")]);
    let error = run(
        &fixture.call(TargetProvider::Gemini, Execution::Sync),
        pooled_only(&pooled),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        GeminiEmbeddingCallError::MetadataShape { .. }
    ));
    assert_eq!(pooled.sent().len(), 1);
}

#[tokio::test]
async fn optional_params_reach_the_request_transform() {
    let mut fixture = Fixture::new(r#""hello""#);
    fixture.optional_params = [(String::from("title"), json("1"))].into_iter().collect();
    let pooled = Script::default();
    let error = run(
        &fixture.call(TargetProvider::Vertex, Execution::Sync),
        pooled_only(&pooled),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, GeminiEmbeddingCallError::Records(_)));
    assert!(pooled.sent().is_empty());
}
