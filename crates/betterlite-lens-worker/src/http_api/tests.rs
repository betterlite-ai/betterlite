#![cfg(test)]

use std::collections::BTreeMap;
use std::future::pending;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_lens::models::decode::decode;
use betterlite_lens::models::execution::Coverage;
use betterlite_lens::models::messages::{Claim, JobResult, ModelRequest, Progress, Purpose};
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::api::{LensApi, WorkerError};
use crate::http_api::HttpApi;

struct Recorded {
    head: String,
    body: Vec<u8>,
}

struct Harness {
    api: HttpApi,
    listener: TcpListener,
    tasks: JoinSet<Recorded>,
}

async fn harness(suffix: &str) -> Harness {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let client = ProxiedClient::outbound(&policy).unwrap();
    let api = HttpApi::new(
        &format!("http://{address}{suffix}"),
        SecretString::from("token"),
        client,
    );
    Harness {
        api,
        listener,
        tasks: JoinSet::new(),
    }
}

async fn answer(listener: TcpListener, status: &'static str, content: Vec<u8>) -> Recorded {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        let mut byte = [0_u8; 1];
        stream.read_exact(&mut byte).await.unwrap();
        received.push(byte[0]);
    }
    let head = String::from_utf8(received).unwrap();
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .map_or(0, |value| value.trim().parse::<usize>().unwrap());
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body).await.unwrap();
    let reply = format!(
        "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        content.len()
    );
    stream.write_all(reply.as_bytes()).await.unwrap();
    stream.write_all(&content).await.unwrap();
    Recorded { head, body }
}

async fn hold(listener: TcpListener) {
    let (_stream, _) = listener.accept().await.unwrap();
    pending::<()>().await;
}

async fn claim(api: &HttpApi) -> Result<Option<Claim>, WorkerError> {
    LensApi::claim(api).await
}

fn sample_json() -> Vec<u8> {
    String::from(
        r#"{"executions":[{"id":"exec","source":"traces","trace_id":"trace","team_id":"team","name":"run","start_time":"2026-01-01T00:00:00Z","span_count":1}],"eligible":1}"#,
    )
    .into_bytes()
}

fn content_json() -> Vec<u8> {
    String::from(
        r#"{"execution":{"id":"exec","source":"traces","trace_id":"trace","team_id":"team","name":"run","start_time":"2026-01-01T00:00:00Z","span_count":1},"parts":[{"execution_id":"exec","span_id":"s0","name":"root","kind":"agent","content":"content"}]}"#,
    )
    .into_bytes()
}

fn request() -> ModelRequest {
    ModelRequest {
        prompt: String::from("look"),
        purpose: Purpose::Extract,
    }
}

#[tokio::test]
async fn claims_with_the_protocol_version_and_the_bearer_header() {
    let mut harness = harness("").await;
    harness
        .tasks
        .spawn(answer(harness.listener, "200 OK", b"null".to_vec()));
    let claimed = claim(&harness.api).await.unwrap();
    let recorded = harness.tasks.join_next().await.unwrap().unwrap();
    assert!(claimed.is_none());
    assert_eq!(
        recorded.head.lines().next().unwrap(),
        "POST /lens/worker/claim?protocol_version=2 HTTP/1.1"
    );
    assert!(recorded.head.contains("authorization: Bearer token"));
    assert!(recorded.body.is_empty());
}

#[tokio::test]
async fn claim_decodes_a_claimed_job() {
    let mut harness = harness("").await;
    harness.tasks.spawn(answer(
        harness.listener,
        "200 OK",
        include_bytes!("../fixtures/claim.json").to_vec(),
    ));
    let claimed = claim(&harness.api).await.unwrap().unwrap();
    assert_eq!(claimed.lens_id, "lens");
    assert_eq!(claimed.job.id, "job");
}

#[tokio::test]
async fn sample_reads_its_path() {
    let mut harness = harness("").await;
    harness
        .tasks
        .spawn(answer(harness.listener, "200 OK", sample_json()));
    let sample = harness.api.sample("/lens/worker/lens/job").await.unwrap();
    let recorded = harness.tasks.join_next().await.unwrap().unwrap();
    assert_eq!(
        recorded.head.lines().next().unwrap(),
        "GET /lens/worker/lens/job/sample HTTP/1.1"
    );
    assert_eq!(sample.eligible, 1);
}

#[tokio::test]
async fn content_escapes_the_query_parameters() {
    let mut harness = harness("").await;
    harness
        .tasks
        .spawn(answer(harness.listener, "200 OK", content_json()));
    let content = harness
        .api
        .content("/lens/worker/lens/job", "exec", "a&b c", 7)
        .await
        .unwrap();
    let recorded = harness.tasks.join_next().await.unwrap().unwrap();
    assert_eq!(
        recorded.head.lines().next().unwrap(),
        "GET /lens/worker/lens/job/content?execution_id=exec&cursor=a%26b+c&offset=7 HTTP/1.1"
    );
    assert_eq!(content.parts.len(), 1);
}

#[tokio::test]
async fn model_posts_the_request_as_json() {
    let mut harness = harness("").await;
    harness.tasks.spawn(answer(
        harness.listener,
        "200 OK",
        br#"{"content":"hello","cost":0.25}"#.to_vec(),
    ));
    let request = request();
    let answer = harness
        .api
        .model("/lens/worker/lens/job", request.clone())
        .await
        .unwrap();
    let recorded = harness.tasks.join_next().await.unwrap().unwrap();
    assert_eq!(
        recorded.head.lines().next().unwrap(),
        "POST /lens/worker/lens/job/model HTTP/1.1"
    );
    assert!(recorded.head.contains("content-type: application/json"));
    assert_eq!(decode::<ModelRequest>(&recorded.body).unwrap(), request);
    assert_eq!(answer.content, "hello");
}

#[tokio::test]
async fn progress_posts_the_stage_and_coverage() {
    let mut harness = harness("").await;
    harness
        .tasks
        .spawn(answer(harness.listener, "200 OK", Vec::new()));
    let coverage = Coverage {
        eligible: 3,
        selected: 2,
        screened: 1,
        investigated: 0,
        inconclusive: 0,
        grouping_batches: 0,
        grouped_batches: 0,
        candidates: 0,
        partial: 0,
        unassessable: 0,
    };
    harness
        .api
        .progress(
            "/lens/worker/lens/job",
            Progress {
                stage: String::from("screening"),
                coverage: coverage.clone(),
            },
        )
        .await
        .unwrap();
    let recorded = harness.tasks.join_next().await.unwrap().unwrap();
    assert_eq!(
        recorded.head.lines().next().unwrap(),
        "POST /lens/worker/lens/job/progress HTTP/1.1"
    );
    let posted = decode::<Progress>(&recorded.body).unwrap();
    assert_eq!(posted.stage, "screening");
    assert_eq!(posted.coverage, coverage);
}

#[tokio::test]
async fn heartbeat_posts_without_a_body() {
    let mut harness = harness("").await;
    harness
        .tasks
        .spawn(answer(harness.listener, "200 OK", Vec::new()));
    harness
        .api
        .heartbeat("/lens/worker/lens/job")
        .await
        .unwrap();
    let recorded = harness.tasks.join_next().await.unwrap().unwrap();
    assert_eq!(
        recorded.head.lines().next().unwrap(),
        "POST /lens/worker/lens/job/heartbeat HTTP/1.1"
    );
    assert!(recorded.body.is_empty());
}

#[tokio::test]
async fn result_posts_the_job_result_and_returns_its_status() {
    let mut harness = harness("").await;
    harness
        .tasks
        .spawn(answer(harness.listener, "202 Accepted", Vec::new()));
    let result = JobResult {
        assessments: Vec::new(),
        findings: Vec::new(),
        coverage: Coverage::default(),
        error: String::new(),
    };
    let status = harness
        .api
        .result("/lens/worker/lens/job", &result)
        .await
        .unwrap();
    let recorded = harness.tasks.join_next().await.unwrap().unwrap();
    assert_eq!(
        recorded.head.lines().next().unwrap(),
        "POST /lens/worker/lens/job/result HTTP/1.1"
    );
    assert_eq!(decode::<JobResult>(&recorded.body).unwrap(), result);
    assert_eq!(status.status, 202);
}

#[tokio::test]
async fn status_errors_drop_the_query_from_the_path() {
    let mut harness = harness("").await;
    harness.tasks.spawn(answer(
        harness.listener,
        "500 Internal Server Error",
        Vec::new(),
    ));
    let error = claim(&harness.api).await.unwrap_err();
    assert!(matches!(
        error,
        WorkerError::Status { path, status: 500 } if path == "/lens/worker/claim"
    ));
}

#[tokio::test]
async fn status_errors_keep_the_full_request_path() {
    let mut harness = harness("").await;
    harness.tasks.spawn(answer(
        harness.listener,
        "503 Service Unavailable",
        Vec::new(),
    ));
    let error = harness
        .api
        .model("/lens/worker/lens/job", request())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        WorkerError::Status { path, status: 503 } if path == "/lens/worker/lens/job/model"
    ));
}

#[tokio::test]
async fn maps_a_connect_failure_to_transport() {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let client = ProxiedClient::outbound(&policy).unwrap();
    let api = HttpApi::new("http://127.0.0.1:1", SecretString::from("token"), client);
    let error = claim(&api).await.unwrap_err();
    assert!(matches!(error, WorkerError::Transport));
}

#[tokio::test(start_paused = true)]
async fn refuses_an_unsupported_proxy_without_contacting_the_target() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let environment = BTreeMap::from([(
        String::from("HTTP_PROXY"),
        String::from("socks5://egress:1080"),
    )]);
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &environment).unwrap());
    let client = ProxiedClient::outbound(&policy).unwrap();
    let api = HttpApi::new(
        &format!("http://{address}"),
        SecretString::from("token"),
        client,
    );
    let error = claim(&api).await.unwrap_err();
    assert!(matches!(error, WorkerError::Transport));
    if let Ok(Ok((_stream, _peer))) = timeout(Duration::from_millis(50), listener.accept()).await {
        panic!("the worker reached the target through an unsupported proxy");
    }
}

#[tokio::test(start_paused = true)]
async fn maps_a_request_timeout_to_timeout() {
    let harness = harness("").await;
    let mut pending = JoinSet::new();
    pending.spawn(hold(harness.listener));
    let error = claim(&harness.api).await.unwrap_err();
    assert!(matches!(error, WorkerError::Timeout));
    pending.abort_all();
    while pending.join_next().await.is_some() {}
}

#[tokio::test]
async fn maps_a_malformed_body_to_decode() {
    let mut harness = harness("").await;
    harness
        .tasks
        .spawn(answer(harness.listener, "200 OK", b"not json".to_vec()));
    let error = claim(&harness.api).await.unwrap_err();
    assert!(matches!(error, WorkerError::Decode(_)));
}

#[tokio::test]
async fn trims_trailing_slashes_from_the_base_url() {
    let mut harness = harness("///").await;
    harness
        .tasks
        .spawn(answer(harness.listener, "200 OK", b"null".to_vec()));
    claim(&harness.api).await.unwrap();
    let recorded = harness.tasks.join_next().await.unwrap().unwrap();
    assert_eq!(
        recorded.head.lines().next().unwrap(),
        "POST /lens/worker/claim?protocol_version=2 HTTP/1.1"
    );
}
