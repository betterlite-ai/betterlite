#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::error::UserUrlError;
use betterlite_http::user_url::validate::{Resolve, UrlValidator};
use http::Method;
use rstest::rstest;
use secrecy::SecretString;
use serde::Serialize;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

use super::error::PointFiveUploadError;
use super::types::PointFiveUploadFailure;
use super::upload::PointFiveUploadClient;
use super::upload::sleeper::UploadSleeper;
use super::upload::transport::{PointFiveHttp, UploadHttp, UploadRequest, UploadResponse};
use super::upload::url_check::{UploadUrlCheck, ValidatedUpload};

const API_URL: &str = "https://api.pointfive.test/ingestion";
const PRESIGNED: &str = "https://uploads.example/object?sig=1";
const SECOND_PRESIGNED: &str = "https://uploads.example/object2?sig=2";

#[derive(Debug, Clone)]
enum Reply {
    Answer { status: u16, body: Vec<u8> },
    Transport,
}

impl Reply {
    fn answer(status: u16, body: Vec<u8>) -> Self {
        Self::Answer { status, body }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    method: Method,
    url: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
    follow_redirects: bool,
}

#[derive(Clone, Default)]
struct FakeHttp {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<Mutex<Vec<Seen>>>,
}

impl FakeHttp {
    fn answering(replies: Vec<Reply>) -> Self {
        Self {
            replies: Arc::new(Mutex::new(VecDeque::from(replies))),
            requests: Arc::default(),
        }
    }

    fn requests(&self) -> Vec<Seen> {
        self.requests.lock().expect("the request log").clone()
    }
}

#[async_trait]
impl UploadHttp for FakeHttp {
    async fn send(&self, request: UploadRequest) -> Result<UploadResponse, TransportError> {
        let headers = request
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    value.to_str().expect("a visible header").to_owned(),
                )
            })
            .collect();
        self.requests.lock().expect("the request log").push(Seen {
            method: request.method,
            url: request.url.to_string(),
            headers,
            body: request.body,
            follow_redirects: request.follow_redirects,
        });
        let reply = self.replies.lock().expect("the reply script").pop_front();
        match reply {
            Some(Reply::Answer { status, body }) => Ok(UploadResponse { status, body }),
            Some(Reply::Transport) => Err(TransportError::Closed),
            None => panic!("the test scripted no reply"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FakeResolver;

#[async_trait]
impl Resolve for FakeResolver {
    async fn resolve(&self, host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
        let address = match host {
            "public.test" => IpAddr::from([93, 184, 216, 34]),
            "internal.test" => IpAddr::from([10, 1, 2, 3]),
            _ => return Err(io::Error::other(format!("no address for {host}"))),
        };
        Ok(vec![SocketAddr::new(address, port)])
    }
}

struct FakeUrlCheck {
    host_header: Option<String>,
}

#[async_trait]
impl UploadUrlCheck for FakeUrlCheck {
    async fn check(&self, url: &str) -> Result<ValidatedUpload, UserUrlError> {
        Ok(ValidatedUpload {
            url: url.to_owned(),
            host_header: self.host_header.clone(),
        })
    }
}

#[derive(Clone, Default)]
struct RecordingSleeper {
    delays: Arc<Mutex<Vec<Duration>>>,
}

impl RecordingSleeper {
    fn delays(&self) -> Vec<Duration> {
        self.delays.lock().expect("the sleep log").clone()
    }
}

#[async_trait]
impl UploadSleeper for RecordingSleeper {
    async fn sleep(&self, delay: Duration) {
        self.delays.lock().expect("the sleep log").push(delay);
    }
}

#[derive(Serialize)]
struct TargetBody<'body> {
    #[serde(rename = "uploadUrl")]
    upload_url: &'body str,
    #[serde(rename = "objectKey")]
    object_key: &'body str,
}

fn target_body(upload_url: &str, object_key: &str) -> Vec<u8> {
    serde_json::to_vec(&TargetBody {
        upload_url,
        object_key,
    })
    .expect("a presign body")
}

fn error_body(reason: &str) -> Vec<u8> {
    format!(r#"{{"error":"{reason}"}}"#).into_bytes()
}

fn allow_all(host_header: Option<&str>) -> Arc<dyn UploadUrlCheck> {
    Arc::new(FakeUrlCheck {
        host_header: host_header.map(str::to_owned),
    })
}

fn put_headers(host_header: Option<&str>) -> BTreeMap<String, String> {
    let mut headers = BTreeMap::from([
        ("content-encoding".to_owned(), "gzip".to_owned()),
        ("content-type".to_owned(), "application/x-ndjson".to_owned()),
    ]);
    if let Some(host) = host_header {
        headers.insert("host".to_owned(), host.to_owned());
    }
    headers
}

fn client(
    http: &FakeHttp,
    check: Arc<dyn UploadUrlCheck>,
    sleeper: &RecordingSleeper,
    max_retries: usize,
) -> PointFiveUploadClient {
    PointFiveUploadClient {
        api_key: SecretString::from("key-1"),
        api_url: API_URL.to_owned(),
        http: Arc::new(http.clone()),
        url_check: check,
        sleeper: Arc::new(sleeper.clone()),
        max_retries,
        request_timeout: Duration::from_secs(5),
    }
}

fn proxied_client() -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("an empty proxy policy"),
    );
    ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT)
        .expect("an outbound client")
}

fn real_client(api_url: String) -> PointFiveUploadClient {
    PointFiveUploadClient {
        api_key: SecretString::from("key-1"),
        api_url,
        http: Arc::new(PointFiveHttp {
            client: proxied_client(),
        }),
        url_check: allow_all(None),
        sleeper: Arc::new(RecordingSleeper::default()),
        max_retries: 3,
        request_timeout: Duration::from_secs(5),
    }
}

struct HttpReply {
    status: u16,
    location: Option<&'static str>,
    body: Vec<u8>,
}

impl HttpReply {
    fn status(status: u16) -> Self {
        Self {
            status,
            location: None,
            body: Vec::new(),
        }
    }

    fn redirect(status: u16, location: &'static str) -> Self {
        Self {
            status,
            location: Some(location),
            body: Vec::new(),
        }
    }

    fn json(body: Vec<u8>) -> Self {
        Self {
            status: 200,
            location: None,
            body,
        }
    }
}

fn serve(listener: TcpListener, replies: Vec<HttpReply>) -> JoinSet<Vec<String>> {
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let mut requests = Vec::new();
        for reply in replies {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            requests.push(read_request(&mut stream).await);
            let location = reply
                .location
                .map_or_else(String::new, |value| format!("location: {value}\r\n"));
            let head = format!(
                "HTTP/1.1 {status} X\r\ncontent-length: {length}\r\nconnection: close\r\n{location}\r\n",
                status = reply.status,
                length = reply.body.len()
            );
            stream
                .write_all(head.as_bytes())
                .await
                .expect("a response head");
            stream
                .write_all(&reply.body)
                .await
                .expect("a response body");
            stream.flush().await.expect("a flushed response");
        }
        requests
    });
    tasks
}

async fn read_request(stream: &mut TcpStream) -> String {
    let mut received = Vec::new();
    while !received.windows(4).any(|window| window == b"\r\n\r\n") {
        let mut chunk = [0_u8; 1024];
        let count = stream.read(&mut chunk).await.expect("a request read");
        assert!(count > 0, "the connection closed before the head");
        received.extend_from_slice(chunk.get(..count).expect("a read count"));
    }
    let head_end = received
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("a head end")
        .saturating_add(4);
    let head = String::from_utf8(received.get(..head_end).expect("a head slice").to_vec())
        .expect("a utf-8 head");
    let length = content_length(&head);
    let mut body = received.get(head_end..).expect("a body slice").to_vec();
    while body.len() < length {
        let mut chunk = [0_u8; 1024];
        let count = stream.read(&mut chunk).await.expect("a body read");
        assert!(count > 0, "the connection closed before the body");
        body.extend_from_slice(chunk.get(..count).expect("a read count"));
    }
    format!("{head}{}", String::from_utf8(body).expect("a utf-8 body"))
}

fn content_length(head: &str) -> usize {
    head.lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0)
}

#[test]
fn the_upload_error_renders_its_detail() {
    let error = PointFiveUploadError {
        detail: "pointfive api returned 500, gave up after 3 attempts".to_owned(),
    };
    assert_eq!(
        error.to_string(),
        "pointfive api returned 500, gave up after 3 attempts"
    );
}

#[tokio::test]
async fn upload_presigns_then_puts_the_batch() {
    let http = FakeHttp::answering(vec![
        Reply::answer(200, target_body(PRESIGNED, "objects/1")),
        Reply::answer(200, Vec::new()),
    ]);
    let sleeper = RecordingSleeper::default();
    let outcome = client(&http, allow_all(Some("uploads.example")), &sleeper, 3)
        .upload(b"payload")
        .await;
    assert_eq!(outcome, Ok("objects/1".to_owned()));
    assert!(sleeper.delays().is_empty());
    let requests = http.requests();
    assert_eq!(requests.len(), 2);
    let presign = &requests[0];
    assert_eq!(presign.method, Method::POST);
    assert!(presign.follow_redirects);
    assert_eq!(
        presign.url,
        "https://api.pointfive.test/ingestion/upload-url"
    );
    assert_eq!(
        presign.headers.get("authorization").map(String::as_str),
        Some("Bearer key-1")
    );
    assert_eq!(
        presign.headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        presign.body,
        br#"{"kind":"LITELLM","byteCount":7}"#.to_vec()
    );
    let put = &requests[1];
    assert_eq!(put.method, Method::PUT);
    assert!(!put.follow_redirects);
    assert_eq!(put.url, PRESIGNED);
    assert_eq!(put.body, b"payload".to_vec());
    assert_eq!(put.headers, put_headers(Some("uploads.example")));
}

#[tokio::test]
async fn the_put_omits_the_host_header_when_validation_gives_none() {
    let http = FakeHttp::answering(vec![
        Reply::answer(200, target_body(PRESIGNED, "objects/1")),
        Reply::answer(200, Vec::new()),
    ]);
    let outcome = client(&http, allow_all(None), &RecordingSleeper::default(), 3)
        .upload(b"payload")
        .await;
    assert_eq!(outcome, Ok("objects/1".to_owned()));
    let requests = http.requests();
    let put = &requests[1];
    assert_eq!(put.headers, put_headers(None));
}

#[tokio::test]
async fn an_unreadable_presign_body_is_a_non_retryable_failure() {
    let http = FakeHttp::answering(vec![Reply::answer(200, b"not json".to_vec())]);
    let outcome = client(&http, allow_all(None), &RecordingSleeper::default(), 3)
        .upload(b"payload")
        .await;
    assert_eq!(
        outcome,
        Err(PointFiveUploadFailure {
            detail: "pointfive api returned an unreadable body".to_owned(),
            retryable: false,
        })
    );
    assert_eq!(http.requests().len(), 1);
}

#[rstest]
#[case::rate_limited(429, true)]
#[case::server_error(500, true)]
#[case::bad_gateway(502, true)]
#[case::unavailable(503, true)]
#[case::gateway_timeout(504, true)]
#[case::bad_request(400, false)]
#[case::unauthorized(401, false)]
#[case::not_found(404, false)]
#[case::not_implemented(501, false)]
#[tokio::test]
async fn presign_status_codes_retry_only_for_the_configured_set(
    #[case] status: u16,
    #[case] retryable: bool,
) {
    let http = FakeHttp::answering(vec![
        Reply::answer(status, error_body("refused")),
        Reply::answer(status, error_body("refused")),
    ]);
    let sleeper = RecordingSleeper::default();
    let outcome = client(&http, allow_all(None), &sleeper, 2)
        .upload(b"payload")
        .await;
    let failure = outcome.expect_err("the presign is refused");
    if retryable {
        assert!(failure.retryable);
        assert_eq!(http.requests().len(), 2);
        assert_eq!(sleeper.delays(), vec![Duration::from_secs(1)]);
        assert_eq!(
            failure.detail,
            format!("pointfive api returned {status}, refused, gave up after 2 attempts")
        );
    } else {
        assert!(!failure.retryable);
        assert_eq!(http.requests().len(), 1);
        assert!(sleeper.delays().is_empty());
        assert_eq!(
            failure.detail,
            format!("pointfive api returned {status}, refused")
        );
    }
}

#[tokio::test]
async fn a_presign_failure_without_a_reason_reports_only_the_status() {
    let http = FakeHttp::answering(vec![Reply::answer(418, b"<html>teapot</html>".to_vec())]);
    let outcome = client(&http, allow_all(None), &RecordingSleeper::default(), 3)
        .upload(b"payload")
        .await;
    assert_eq!(
        outcome,
        Err(PointFiveUploadFailure {
            detail: "pointfive api returned 418".to_owned(),
            retryable: false,
        })
    );
    assert_eq!(http.requests().len(), 1);
}

#[tokio::test]
async fn a_put_status_outside_the_retry_set_is_not_retried() {
    let http = FakeHttp::answering(vec![
        Reply::answer(200, target_body(PRESIGNED, "objects/1")),
        Reply::answer(400, error_body("rejected")),
    ]);
    let sleeper = RecordingSleeper::default();
    let outcome = client(&http, allow_all(None), &sleeper, 3)
        .upload(b"payload")
        .await;
    assert_eq!(
        outcome,
        Err(PointFiveUploadFailure {
            detail: "presigned upload returned 400, rejected".to_owned(),
            retryable: false,
        })
    );
    assert_eq!(http.requests().len(), 2);
    assert!(sleeper.delays().is_empty());
}

#[rstest]
#[case::moved_permanently(301)]
#[case::found(302)]
#[case::see_other(303)]
#[case::temporary_redirect(307)]
#[case::permanent_redirect(308)]
#[tokio::test]
async fn a_redirected_put_is_refused(#[case] status: u16) {
    let http = FakeHttp::answering(vec![
        Reply::answer(200, target_body(PRESIGNED, "objects/1")),
        Reply::answer(status, Vec::new()),
    ]);
    let sleeper = RecordingSleeper::default();
    let outcome = client(&http, allow_all(None), &sleeper, 3)
        .upload(b"payload")
        .await;
    assert_eq!(
        outcome,
        Err(PointFiveUploadFailure {
            detail: format!("presigned upload redirected with {status}, refusing to follow"),
            retryable: false,
        })
    );
    assert_eq!(http.requests().len(), 2);
    assert!(sleeper.delays().is_empty());
}

#[tokio::test]
async fn the_presign_post_follows_a_redirect() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a listener");
    let address = listener.local_addr().expect("an address");
    let presigned = format!("http://{address}/upload");
    let replies = vec![
        HttpReply::redirect(307, "/ingestion/moved"),
        HttpReply::json(target_body(&presigned, "objects/1")),
        HttpReply::status(200),
    ];
    let mut tasks = serve(listener, replies);
    let client = real_client(format!("http://{address}/ingestion"));
    let outcome = client.upload(b"payload").await;
    assert_eq!(outcome, Ok("objects/1".to_owned()));
    let requests = tasks
        .join_next()
        .await
        .expect("the server task")
        .expect("the served requests");
    assert_eq!(requests.len(), 3);
    assert!(
        requests[0].starts_with("POST /ingestion/upload-url "),
        "{}",
        requests[0]
    );
    assert!(
        requests[1].starts_with("POST /ingestion/moved "),
        "{}",
        requests[1]
    );
    assert!(
        requests[1].contains(r#"{"kind":"LITELLM","byteCount":7}"#),
        "{}",
        requests[1]
    );
    assert!(requests[2].starts_with("PUT /upload "), "{}", requests[2]);
}

#[tokio::test]
async fn the_presigned_put_does_not_follow_a_redirect() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("a listener");
    let address = listener.local_addr().expect("an address");
    let presigned = format!("http://{address}/upload");
    let replies = vec![
        HttpReply::json(target_body(&presigned, "objects/1")),
        HttpReply::redirect(307, "/elsewhere"),
    ];
    let mut tasks = serve(listener, replies);
    let client = real_client(format!("http://{address}/ingestion"));
    let outcome = client.upload(b"payload").await;
    assert_eq!(
        outcome,
        Err(PointFiveUploadFailure {
            detail: "presigned upload redirected with 307, refusing to follow".to_owned(),
            retryable: false,
        })
    );
    let requests = tasks
        .join_next()
        .await
        .expect("the server task")
        .expect("the served requests");
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0].starts_with("POST /ingestion/upload-url "),
        "{}",
        requests[0]
    );
    assert!(requests[1].starts_with("PUT /upload "), "{}", requests[1]);
}

#[tokio::test]
async fn a_retryable_put_presigns_again_and_succeeds() {
    let http = FakeHttp::answering(vec![
        Reply::answer(200, target_body(PRESIGNED, "objects/1")),
        Reply::answer(503, error_body("busy")),
        Reply::answer(200, target_body(SECOND_PRESIGNED, "objects/2")),
        Reply::answer(200, Vec::new()),
    ]);
    let sleeper = RecordingSleeper::default();
    let outcome = client(&http, allow_all(None), &sleeper, 3)
        .upload(b"payload")
        .await;
    assert_eq!(outcome, Ok("objects/2".to_owned()));
    let requests = http.requests();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0].method, Method::POST);
    assert_eq!(requests[1].method, Method::PUT);
    assert_eq!(requests[2].method, Method::POST);
    assert_eq!(requests[3].method, Method::PUT);
    assert_eq!(requests[3].url, SECOND_PRESIGNED);
    assert_eq!(requests[0].body, requests[2].body);
    assert_eq!(sleeper.delays(), vec![Duration::from_secs(1)]);
}

#[tokio::test]
async fn every_failed_attempt_backs_off_exponentially_then_gives_up() {
    let http = FakeHttp::answering(vec![
        Reply::answer(200, target_body(PRESIGNED, "objects/1")),
        Reply::answer(500, Vec::new()),
        Reply::answer(200, target_body(PRESIGNED, "objects/1")),
        Reply::answer(500, Vec::new()),
        Reply::answer(200, target_body(PRESIGNED, "objects/1")),
        Reply::answer(500, Vec::new()),
    ]);
    let sleeper = RecordingSleeper::default();
    let outcome = client(&http, allow_all(None), &sleeper, 3)
        .upload(b"payload")
        .await;
    assert_eq!(
        outcome,
        Err(PointFiveUploadFailure {
            detail: "presigned upload returned 500, gave up after 3 attempts".to_owned(),
            retryable: true,
        })
    );
    assert_eq!(http.requests().len(), 6);
    assert_eq!(
        sleeper.delays(),
        vec![Duration::from_secs(1), Duration::from_secs(2)]
    );
}

#[tokio::test]
async fn no_attempt_runs_when_retries_are_zero() {
    let http = FakeHttp::answering(Vec::new());
    let sleeper = RecordingSleeper::default();
    let outcome = client(&http, allow_all(None), &sleeper, 0)
        .upload(b"payload")
        .await;
    assert_eq!(
        outcome,
        Err(PointFiveUploadFailure {
            detail: "max_upload_retries must be at least 1".to_owned(),
            retryable: false,
        })
    );
    assert!(http.requests().is_empty());
    assert!(sleeper.delays().is_empty());
}

#[tokio::test]
async fn a_presign_transport_failure_is_retryable() {
    let http = FakeHttp::answering(vec![Reply::Transport, Reply::Transport]);
    let sleeper = RecordingSleeper::default();
    let outcome = client(&http, allow_all(None), &sleeper, 2)
        .upload(b"payload")
        .await;
    let failure = outcome.expect_err("the api is unreachable");
    assert!(failure.retryable);
    assert!(
        failure.detail.starts_with("pointfive api unreachable:"),
        "{}",
        failure.detail
    );
    assert!(
        failure.detail.ends_with("gave up after 2 attempts"),
        "{}",
        failure.detail
    );
    assert_eq!(http.requests().len(), 2);
    assert_eq!(sleeper.delays(), vec![Duration::from_secs(1)]);
}

#[tokio::test]
async fn a_put_transport_failure_is_retried() {
    let http = FakeHttp::answering(vec![
        Reply::answer(200, target_body(PRESIGNED, "objects/1")),
        Reply::Transport,
        Reply::answer(200, target_body(PRESIGNED, "objects/1")),
        Reply::answer(200, Vec::new()),
    ]);
    let sleeper = RecordingSleeper::default();
    let outcome = client(&http, allow_all(None), &sleeper, 3)
        .upload(b"payload")
        .await;
    assert_eq!(outcome, Ok("objects/1".to_owned()));
    assert_eq!(http.requests().len(), 4);
    assert_eq!(sleeper.delays(), vec![Duration::from_secs(1)]);
}

#[tokio::test]
async fn ping_posts_the_litellm_kind_and_does_not_retry() {
    let http = FakeHttp::answering(vec![Reply::answer(200, Vec::new())]);
    let sleeper = RecordingSleeper::default();
    let outcome = client(&http, allow_all(None), &sleeper, 3).ping().await;
    assert_eq!(outcome, Ok(()));
    assert!(sleeper.delays().is_empty());
    let requests = http.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, Method::POST);
    assert_eq!(requests[0].url, "https://api.pointfive.test/ingestion/ping");
    assert_eq!(
        requests[0].headers.get("authorization").map(String::as_str),
        Some("Bearer key-1")
    );
    assert_eq!(
        requests[0].headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(requests[0].body, br#"{"kind":"LITELLM"}"#.to_vec());
}

#[tokio::test]
async fn ping_ignores_a_success_body() {
    let http = FakeHttp::answering(vec![Reply::answer(200, b"not json".to_vec())]);
    let outcome = client(&http, allow_all(None), &RecordingSleeper::default(), 3)
        .ping()
        .await;
    assert_eq!(outcome, Ok(()));
}

#[tokio::test]
async fn ping_reports_a_status_failure_without_retrying() {
    let http = FakeHttp::answering(vec![Reply::answer(500, error_body("down"))]);
    let sleeper = RecordingSleeper::default();
    let outcome = client(&http, allow_all(None), &sleeper, 3).ping().await;
    assert_eq!(
        outcome,
        Err(PointFiveUploadFailure {
            detail: "pointfive api returned 500, down".to_owned(),
            retryable: true,
        })
    );
    assert_eq!(http.requests().len(), 1);
    assert!(sleeper.delays().is_empty());
}

#[tokio::test]
async fn trailing_slashes_are_stripped_from_the_api_url() {
    let http = FakeHttp::answering(vec![
        Reply::answer(200, target_body(PRESIGNED, "objects/1")),
        Reply::answer(200, Vec::new()),
    ]);
    let client = PointFiveUploadClient {
        api_key: SecretString::from("key-1"),
        api_url: "https://api.pointfive.test/ingestion///".to_owned(),
        http: Arc::new(http.clone()),
        url_check: allow_all(None),
        sleeper: Arc::new(RecordingSleeper::default()),
        max_retries: 3,
        request_timeout: Duration::from_secs(5),
    };
    let outcome = client.upload(b"payload").await;
    assert_eq!(outcome, Ok("objects/1".to_owned()));
    let requests = http.requests();
    assert_eq!(
        requests[0].url,
        "https://api.pointfive.test/ingestion/upload-url"
    );
}

#[tokio::test]
async fn a_blocked_presigned_url_is_refused_before_the_put() {
    let http = FakeHttp::answering(vec![Reply::answer(
        200,
        target_body("http://internal.test/object", "objects/1"),
    )]);
    let check = Arc::new(UrlValidator::new(FakeResolver, &[], &Verification::Enabled));
    let outcome = client(&http, check, &RecordingSleeper::default(), 3)
        .upload(b"payload")
        .await;
    let failure = outcome.expect_err("a blocked address is refused");
    assert!(!failure.retryable);
    assert!(
        failure.detail.starts_with("presigned upload url refused:"),
        "{}",
        failure.detail
    );
    assert_eq!(http.requests().len(), 1);
}

#[tokio::test]
async fn a_malformed_presigned_url_is_refused_without_a_put() {
    let http = FakeHttp::answering(vec![Reply::answer(
        200,
        target_body("http://public.test:99999/object", "objects/1"),
    )]);
    let check = Arc::new(UrlValidator::new(FakeResolver, &[], &Verification::Enabled));
    let outcome = client(&http, check, &RecordingSleeper::default(), 3)
        .upload(b"payload")
        .await;
    let failure = outcome.expect_err("the URL does not parse");
    assert!(!failure.retryable);
    assert!(
        failure
            .detail
            .starts_with("presigned upload url refused: Port out of range"),
        "{}",
        failure.detail
    );
    assert_eq!(http.requests().len(), 1);
}

#[tokio::test]
async fn a_validated_url_is_rewritten_to_its_ip_with_the_original_host() {
    let http = FakeHttp::answering(vec![
        Reply::answer(
            200,
            target_body("http://public.test/object?sig=1", "objects/1"),
        ),
        Reply::answer(200, Vec::new()),
    ]);
    let check = Arc::new(UrlValidator::new(FakeResolver, &[], &Verification::Enabled));
    let outcome = client(&http, check, &RecordingSleeper::default(), 3)
        .upload(b"payload")
        .await;
    assert_eq!(outcome, Ok("objects/1".to_owned()));
    let requests = http.requests();
    assert_eq!(requests[1].url, "http://93.184.216.34/object?sig=1");
    assert_eq!(
        requests[1].headers.get("host").map(String::as_str),
        Some("public.test")
    );
}
