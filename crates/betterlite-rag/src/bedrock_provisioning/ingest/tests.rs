#![cfg(test)]

use std::time::Duration;

use betterlite_core::rag::ingestion::store::{RagStore, StoreRequest};
use rstest::rstest;
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::bedrock_provisioning::ingest::BedrockIngest;
use crate::bedrock_provisioning::provision::tests::support::{
    ScriptedTransport, answer, initialized, new_provisioner,
};

const NO_WAIT: &str =
    r#"{"aws_region_name":"us-west-2","knowledge_base_id":"kb1","s3_bucket":"bucket"}"#;
const WAIT: &str = r#"{"aws_region_name":"us-west-2","knowledge_base_id":"kb1","s3_bucket":"bucket","wait_for_ingestion":true,"ingestion_timeout":300}"#;
const STARTED: &str = r#"{"ingestionJob":{"ingestionJobId":"job1"}}"#;
const STARTING: &str = r#"{"ingestionJob":{"ingestionJobId":"job1","status":"STARTING"}}"#;
const RUNNING: &str = r#"{"ingestionJob":{"ingestionJobId":"job1","status":"IN_PROGRESS"}}"#;
const COMPLETE: &str = r#"{"ingestionJob":{"ingestionJobId":"job1","status":"COMPLETE","statistics":{"numberOfNewDocumentsIndexed":3}}}"#;
const FAILED: &str =
    r#"{"ingestionJob":{"ingestionJobId":"job1","status":"FAILED","failureReasons":["bad file"]}}"#;
const UNKNOWN: &str = r#"{"ingestionJob":{"ingestionJobId":"job1","status":"STOPPING"}}"#;

fn request<'request>(
    content: Option<&'request [u8]>,
    filename: Option<&'request str>,
    content_type: Option<&'request str>,
) -> StoreRequest<'request> {
    StoreRequest {
        file_content: content,
        filename,
        content_type,
        chunks: &[],
        embeddings: None,
        existing_file_id: None,
    }
}

#[tokio::test(start_paused = true)]
async fn store_uploads_then_starts_and_returns_the_key() {
    let transport = ScriptedTransport::new(vec![answer(""), answer(STARTED)]);
    let mut provisioner = initialized(new_provisioner(NO_WAIT, transport.clone()));
    let (knowledge_base, key) = provisioner
        .store(request(
            Some(b"hello"),
            Some("file.txt"),
            Some("text/plain"),
        ))
        .await
        .unwrap();
    assert_eq!(knowledge_base.as_deref(), Some("kb1"));
    assert_eq!(key.as_deref(), Some("data/file.txt"));
    assert_eq!(
        transport.operations(),
        vec!["PutObject", "StartIngestionJob"]
    );
    let put = transport.requests().remove(0);
    assert_eq!(
        put.url,
        "https://bucket.s3.us-west-2.amazonaws.com/data/file.txt"
    );
    assert_eq!(put.body, b"hello".to_vec());
}

#[tokio::test(start_paused = true)]
async fn store_trims_every_trailing_slash_from_the_prefix() {
    let transport = ScriptedTransport::new(vec![answer(""), answer(STARTED)]);
    let mut provisioner = initialized(new_provisioner(NO_WAIT, transport.clone()));
    provisioner.state.s3_prefix = "data//".to_owned();
    let (_, key) = provisioner
        .store(request(Some(b"hello"), Some("file.txt"), None))
        .await
        .unwrap();
    assert_eq!(key.as_deref(), Some("data/file.txt"));
    assert!(!transport.requests().remove(0).url.contains("data//"));
}

#[rstest]
#[case(None, Some("file.txt"))]
#[case(Some(&b""[..]), Some("file.txt"))]
#[case(Some(b"hello"), None)]
#[case(Some(b"hello"), Some(""))]
#[tokio::test(start_paused = true)]
async fn store_returns_the_id_and_none_without_content_or_filename(
    #[case] content: Option<&[u8]>,
    #[case] filename: Option<&str>,
) {
    let transport = ScriptedTransport::new(Vec::new());
    let mut provisioner = initialized(new_provisioner(NO_WAIT, transport.clone()));
    let (knowledge_base, key) = provisioner
        .store(request(content, filename, None))
        .await
        .unwrap();
    assert_eq!(knowledge_base.as_deref(), Some("kb1"));
    assert_eq!(key, None);
    assert!(transport.requests().is_empty());
}

#[tokio::test(start_paused = true)]
async fn store_returns_no_id_when_the_knowledge_base_is_unknown() {
    let transport = ScriptedTransport::new(Vec::new());
    let mut provisioner = initialized(new_provisioner(
        r#"{"aws_region_name":"us-west-2","s3_bucket":"bucket"}"#,
        transport,
    ));
    let (knowledge_base, key) = provisioner
        .store(request(None, Some("file.txt"), None))
        .await
        .unwrap();
    assert_eq!(knowledge_base, None);
    assert_eq!(key, None);
}

#[tokio::test(start_paused = true)]
async fn store_polls_until_complete() {
    let transport = ScriptedTransport::new(vec![
        answer(""),
        answer(STARTED),
        answer(STARTING),
        answer(RUNNING),
        answer(COMPLETE),
    ]);
    let mut provisioner = initialized(new_provisioner(WAIT, transport.clone()));
    let started = Instant::now();
    let (_, key) = provisioner
        .store(request(Some(b"hello"), Some("file.txt"), None))
        .await
        .unwrap();
    assert_eq!(key.as_deref(), Some("data/file.txt"));
    assert_eq!(
        transport.operations(),
        vec![
            "PutObject",
            "StartIngestionJob",
            "GetIngestionJob",
            "GetIngestionJob",
            "GetIngestionJob",
        ]
    );
    assert_eq!(started.elapsed(), Duration::from_secs(4));
}

#[tokio::test(start_paused = true)]
async fn store_stops_on_a_failed_job() {
    let transport = ScriptedTransport::new(vec![answer(""), answer(STARTED), answer(FAILED)]);
    let mut provisioner = initialized(new_provisioner(WAIT, transport.clone()));
    let started = Instant::now();
    provisioner
        .store(request(Some(b"hello"), Some("file.txt"), None))
        .await
        .unwrap();
    assert_eq!(started.elapsed(), Duration::ZERO);
    assert_eq!(transport.requests().len(), 3);
}

#[tokio::test(start_paused = true)]
async fn store_stops_on_an_unknown_status() {
    let transport = ScriptedTransport::new(vec![answer(""), answer(STARTED), answer(UNKNOWN)]);
    let mut provisioner = initialized(new_provisioner(WAIT, transport.clone()));
    provisioner
        .store(request(Some(b"hello"), Some("file.txt"), None))
        .await
        .unwrap();
    assert_eq!(transport.requests().len(), 3);
}

#[tokio::test(start_paused = true)]
async fn store_runs_out_of_time_silently() {
    let mut answers = vec![answer(""), answer(STARTED)];
    answers.extend(std::iter::repeat_n(answer(STARTING), 5));
    let transport = ScriptedTransport::new(answers);
    let mut provisioner = initialized(new_provisioner(
        r#"{"aws_region_name":"us-west-2","knowledge_base_id":"kb1","s3_bucket":"bucket","wait_for_ingestion":true,"ingestion_timeout":5}"#,
        transport.clone(),
    ));
    let started = Instant::now();
    let (_, key) = provisioner
        .store(request(Some(b"hello"), Some("file.txt"), None))
        .await
        .unwrap();
    assert_eq!(key.as_deref(), Some("data/file.txt"));
    assert_eq!(started.elapsed(), Duration::from_secs(6));
    assert_eq!(transport.requests().len(), 5);
}

#[tokio::test(start_paused = true)]
async fn store_reports_a_missing_bucket() {
    let transport = ScriptedTransport::new(Vec::new());
    let mut provisioner = initialized(new_provisioner(NO_WAIT, transport));
    provisioner.state.s3_bucket = None;
    assert!(
        provisioner
            .store(request(Some(b"hello"), Some("file.txt"), None))
            .await
            .is_err()
    );
}

#[tokio::test(start_paused = true)]
async fn store_reports_a_missing_data_source() {
    let transport = ScriptedTransport::new(Vec::new());
    let mut provisioner = initialized(new_provisioner(NO_WAIT, transport));
    provisioner.state.data_source_id = None;
    assert!(
        provisioner
            .store(request(Some(b"hello"), Some("file.txt"), None))
            .await
            .is_err()
    );
}

#[test]
fn store_embed_returns_none_for_every_chunk() {
    let transport = ScriptedTransport::new(Vec::new());
    let provisioner = initialized(new_provisioner(NO_WAIT, transport));
    let chunks = vec!["first chunk".to_owned(), "second chunk".to_owned()];
    assert_eq!(provisioner.embed(&chunks), None);
}

#[tokio::test(start_paused = true)]
async fn store_trait_forwards_to_the_provisioner() {
    let transport = ScriptedTransport::new(vec![answer(""), answer(STARTED)]);
    let ingest = BedrockIngest {
        provisioner: Mutex::new(initialized(new_provisioner(NO_WAIT, transport.clone()))),
    };
    let (knowledge_base, key) = RagStore::store(
        &ingest,
        request(Some(b"hello"), Some("file.txt"), Some("text/plain")),
    )
    .await
    .unwrap();
    assert_eq!(knowledge_base.as_deref(), Some("kb1"));
    assert_eq!(key.as_deref(), Some("data/file.txt"));
    assert_eq!(
        transport.operations(),
        vec!["PutObject", "StartIngestionJob"]
    );
}
