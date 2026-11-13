#![cfg(test)]

use std::collections::VecDeque;
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use http::Method;
use md5::Md5;
use rstest::rstest;
use secrecy::SecretString;
use sha2::{Digest as _, Sha256};
use tokio::time::Instant;

use crate::s3_transport::batch_element::{ContentType, S3BatchLoggingElement};
use crate::s3_transport::encryption::ServerSideEncryption;
use crate::s3_transport::error::S3TransportError;
use crate::s3_transport::signing::{
    Clock, CredentialError, CredentialSource, Credentials, RequestToSign, StaticCredentials,
    sign_request,
};
use crate::s3_transport::transport::{
    S3HttpClient, S3Request, S3Response, S3Transport, ThrottleSink, UploadOutcome,
};
use crate::s3_transport::url_construction::S3Endpoint;

const PUT_AUTHORIZATION: &str = "AWS4-HMAC-SHA256 Credential=AKIAEXAMPLE/20260102/us-east-1/s3/aws4_request, SignedHeaders=cache-control;content-disposition;content-language;content-md5;content-type;host;x-amz-content-sha256;x-amz-date;x-amz-security-token;x-amz-server-side-encryption;x-amz-server-side-encryption-aws-kms-key-id, Signature=48ba162a8b777dd7bcd7826480fcab9d6195de43c5c6486902800481df4c1656";
const ID_JSON: &str = "{\"id\":\"request\"}";

#[derive(Clone, Debug)]
enum Reply {
    Response(u16, String),
    Failure,
}

#[derive(Clone, Debug, Default)]
struct FakeS3 {
    state: Arc<FakeState>,
}

#[derive(Debug, Default)]
struct FakeState {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<(S3Request, Instant)>>,
}

impl FakeS3 {
    fn with_replies(replies: Vec<Reply>) -> Self {
        let fake = Self::default();
        fake.state.replies.lock().expect("replies").extend(replies);
        fake
    }

    fn requests(&self) -> Vec<S3Request> {
        self.state
            .requests
            .lock()
            .expect("requests")
            .iter()
            .map(|(request, _)| request.clone())
            .collect()
    }

    fn instants(&self) -> Vec<Instant> {
        let state = self.state.requests.lock().expect("requests");
        let mut instants = Vec::with_capacity(state.len());
        for (_, instant) in state.iter() {
            instants.push(*instant);
        }
        drop(state);
        instants
    }
}

impl S3HttpClient for FakeS3 {
    fn send(
        &self,
        request: S3Request,
    ) -> Pin<Box<dyn Future<Output = Result<S3Response, S3TransportError>> + Send + '_>> {
        Box::pin(async move {
            self.state
                .requests
                .lock()
                .expect("requests")
                .push((request, Instant::now()));
            let reply = self.state.replies.lock().expect("replies").pop_front();
            match reply {
                Some(Reply::Response(status, body)) => Ok(S3Response {
                    status,
                    body: body.into_bytes(),
                }),
                Some(Reply::Failure) => Err(S3TransportError::Send(TransportError::Closed)),
                None => Ok(S3Response {
                    status: 200,
                    body: Vec::new(),
                }),
            }
        })
    }
}

#[derive(Debug, Default)]
struct Generations {
    calls: AtomicUsize,
}

impl Generations {
    fn next_generation(&self) -> Credentials {
        let generation = self.calls.fetch_add(1, Ordering::SeqCst).saturating_add(1);
        Credentials {
            access_key_id: SecretString::from(format!("AKIA{generation}")),
            secret_access_key: SecretString::from(format!("secret{generation}")),
            session_token: Some(SecretString::from(format!("token{generation}"))),
        }
    }
}

impl CredentialSource for Generations {
    fn resolve(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Credentials, CredentialError>> + Send + '_>> {
        Box::pin(ready(Ok(self.next_generation())))
    }
}

#[derive(Debug)]
struct FixedClock(SystemTime);

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        self.0
    }
}

#[derive(Clone, Debug, Default)]
struct Samples {
    recorded: Arc<Mutex<Vec<bool>>>,
}

impl Samples {
    fn recorded(&self) -> Vec<bool> {
        self.recorded.lock().expect("samples").clone()
    }
}

impl ThrottleSink for Samples {
    fn record(&self, throttled: bool) {
        self.recorded.lock().expect("samples").push(throttled);
    }
}

fn fixed_time() -> SystemTime {
    let stamp =
        chrono::DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z").expect("the time parses");
    let seconds = u64::try_from(stamp.timestamp()).expect("the time is after the epoch");
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .expect("the time fits")
}

fn endpoint() -> S3Endpoint {
    S3Endpoint {
        bucket: "logs".to_owned(),
        region: "us-east-1".to_owned(),
        endpoint_url: Some("http://127.0.0.1:9000".to_owned()),
        virtual_hosted_style: false,
    }
}

fn static_credentials() -> StaticCredentials {
    StaticCredentials {
        credentials: Credentials {
            access_key_id: SecretString::from("AKIAEXAMPLE"),
            secret_access_key: SecretString::from("secretKeyEXAMPLE"),
            session_token: Some(SecretString::from("tokenEXAMPLE")),
        },
    }
}

fn transport<Credential>(client: &FakeS3, credentials: Arc<Credential>) -> S3Transport
where
    Credential: CredentialSource + 'static,
{
    S3Transport {
        client: Arc::new(client.clone()),
        endpoint: endpoint(),
        credentials,
        encryption: None,
        drop_on_terminal_error: true,
        throttle: None,
        clock: Arc::new(FixedClock(fixed_time())),
    }
}

fn element(object_key: &str, body: &str) -> S3BatchLoggingElement<Json> {
    S3BatchLoggingElement {
        payload: Json::Null,
        s3_object_key: object_key.to_owned(),
        s3_object_download_filename: "download.json".to_owned(),
        body: Some(body.to_owned()),
        content_type: ContentType::Json,
        retrying_since: None,
    }
}

fn header<'request>(request: &'request S3Request, name: &str) -> Option<&'request str> {
    request
        .headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn credential_scope(request: &S3Request) -> &str {
    let authorization = header(request, "authorization").expect("the request is signed");
    authorization
        .split_once("Credential=")
        .and_then(|(_, rest)| rest.split_once('/'))
        .map(|(access_key, _)| access_key)
        .expect("the authorization names the access key")
}

#[tokio::test]
async fn put_carries_the_prepared_headers_and_body() {
    let client = FakeS3::default();
    let transport = transport(&client, Arc::new(static_credentials()));
    let outcome = transport
        .async_upload_data_to_s3(&element("2026-01-02/id.json", ID_JSON))
        .await;
    assert_eq!(outcome, UploadOutcome::Delivered);
    let requests = client.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, Method::PUT);
    assert_eq!(request.url, "http://127.0.0.1:9000/logs/2026-01-02/id.json");
    assert_eq!(request.body, ID_JSON.as_bytes());
    assert_eq!(header(request, "content-type"), Some("application/json"));
    assert_eq!(
        header(request, "content-md5"),
        Some(STANDARD.encode(Md5::digest(ID_JSON.as_bytes())).as_str())
    );
    assert_eq!(
        header(request, "x-amz-content-sha256"),
        Some(hex::encode(Sha256::digest(ID_JSON.as_bytes())).as_str())
    );
    assert_eq!(header(request, "content-language"), Some("en"));
    assert_eq!(
        header(request, "content-disposition"),
        Some("inline; filename=\"download.json\"")
    );
    assert_eq!(
        header(request, "cache-control"),
        Some("private, immutable, max-age=31536000, s-maxage=0")
    );
    assert_eq!(header(request, "x-amz-date"), Some("20260102T030405Z"));
    assert_eq!(
        header(request, "x-amz-security-token"),
        Some("tokenEXAMPLE")
    );
    assert_eq!(credential_scope(request), "AKIAEXAMPLE");
}

#[tokio::test]
async fn the_put_signature_matches_botocore() {
    let client = FakeS3::default();
    let mut transport = transport(&client, Arc::new(static_credentials()));
    transport.encryption = Some(ServerSideEncryption {
        algorithm: "aws:kms".to_owned(),
        kms_key_id: Some("key-id".to_owned()),
    });
    let outcome = transport
        .async_upload_data_to_s3(&element("2026-01-02/id.json", ID_JSON))
        .await;
    assert_eq!(outcome, UploadOutcome::Delivered);
    let requests = client.requests();
    assert_eq!(
        header(&requests[0], "authorization"),
        Some(PUT_AUTHORIZATION)
    );
}

#[rstest]
#[case::ndjson(ContentType::Ndjson, "application/x-ndjson")]
#[case::json(ContentType::Json, "application/json")]
#[tokio::test]
async fn the_content_type_comes_from_the_element(
    #[case] content_type: ContentType,
    #[case] expected: &str,
) {
    let client = FakeS3::default();
    let transport = transport(&client, Arc::new(static_credentials()));
    let mut log_element = element("2026-01-02/id.json", ID_JSON);
    log_element.content_type = content_type;
    transport.async_upload_data_to_s3(&log_element).await;
    let requests = client.requests();
    assert_eq!(header(&requests[0], "content-type"), Some(expected));
}

#[rstest]
#[case::none(None, None, None)]
#[case::algorithm_only(
    Some(ServerSideEncryption { algorithm: "AES256".to_owned(), kms_key_id: None }),
    Some("AES256"),
    None
)]
#[case::algorithm_and_key(
    Some(ServerSideEncryption { algorithm: "aws:kms".to_owned(), kms_key_id: Some("key-id".to_owned()) }),
    Some("aws:kms"),
    Some("key-id")
)]
#[case::empty_algorithm_keeps_the_key(
    Some(ServerSideEncryption { algorithm: String::new(), kms_key_id: Some("key-id".to_owned()) }),
    None,
    Some("key-id")
)]
#[tokio::test]
async fn sse_headers_are_added_only_when_configured(
    #[case] encryption: Option<ServerSideEncryption>,
    #[case] algorithm: Option<&str>,
    #[case] kms_key_id: Option<&str>,
) {
    let client = FakeS3::default();
    let mut transport = transport(&client, Arc::new(static_credentials()));
    transport.encryption = encryption;
    transport
        .async_upload_data_to_s3(&element("2026-01-02/id.json", ID_JSON))
        .await;
    let requests = client.requests();
    assert_eq!(
        header(&requests[0], "x-amz-server-side-encryption"),
        algorithm
    );
    assert_eq!(
        header(&requests[0], "x-amz-server-side-encryption-aws-kms-key-id"),
        kms_key_id
    );
}

#[tokio::test(start_paused = true)]
async fn retryable_statuses_are_retried_with_exponential_backoff() {
    let client = FakeS3::with_replies(vec![
        Reply::Response(500, String::new()),
        Reply::Response(503, String::new()),
        Reply::Response(500, String::new()),
    ]);
    let transport = transport(&client, Arc::new(static_credentials()));
    let outcome = transport
        .async_upload_data_to_s3(&element("2026-01-02/id.json", ID_JSON))
        .await;
    assert_eq!(outcome, UploadOutcome::Retry);
    assert_eq!(client.requests().len(), 3);
    let instants = client.instants();
    assert_eq!(
        instants[1].duration_since(instants[0]),
        Duration::from_secs(1)
    );
    assert_eq!(
        instants[2].duration_since(instants[1]),
        Duration::from_secs(2)
    );
}

#[tokio::test(start_paused = true)]
async fn a_retry_that_succeeds_is_delivered() {
    let client = FakeS3::with_replies(vec![
        Reply::Response(500, String::new()),
        Reply::Response(200, String::new()),
    ]);
    let transport = transport(&client, Arc::new(static_credentials()));
    let outcome = transport
        .async_upload_data_to_s3(&element("2026-01-02/id.json", ID_JSON))
        .await;
    assert_eq!(outcome, UploadOutcome::Delivered);
    assert_eq!(client.requests().len(), 2);
}

#[rstest]
#[case::forbidden(403)]
#[case::server_error(500)]
#[case::unavailable(503)]
#[tokio::test(start_paused = true)]
async fn only_403_500_and_503_are_retried(#[case] status: u16) {
    let client = FakeS3::with_replies(vec![Reply::Response(status, String::new()); 3]);
    let transport = transport(&client, Arc::new(static_credentials()));
    let outcome = transport
        .async_upload_data_to_s3(&element("2026-01-02/id.json", ID_JSON))
        .await;
    assert_eq!(outcome, UploadOutcome::Retry);
    assert_eq!(client.requests().len(), 3);
}

#[rstest]
#[case::bad_request(400)]
#[case::unauthorized(401)]
#[case::not_found(404)]
#[case::too_many_requests(429)]
#[case::bad_gateway(502)]
#[tokio::test(start_paused = true)]
async fn other_statuses_end_the_upload_after_one_attempt(#[case] status: u16) {
    let client = FakeS3::with_replies(vec![Reply::Response(status, String::new()); 3]);
    let transport = transport(&client, Arc::new(static_credentials()));
    let outcome = transport
        .async_upload_data_to_s3(&element("2026-01-02/id.json", ID_JSON))
        .await;
    assert_eq!(outcome, UploadOutcome::Retry);
    assert_eq!(client.requests().len(), 1);
}

#[rstest]
#[case::moved_permanently(301)]
#[case::temporary_redirect(307)]
#[case::not_modified(304)]
#[tokio::test]
async fn redirects_are_not_a_delivery(#[case] status: u16) {
    let client = FakeS3::with_replies(vec![Reply::Response(status, String::new()); 3]);
    let transport = transport(&client, Arc::new(static_credentials()));
    let outcome = transport
        .async_upload_data_to_s3(&element("2026-01-02/id.json", ID_JSON))
        .await;
    assert_eq!(outcome, UploadOutcome::Retry);
    assert_eq!(client.requests().len(), 1);
}

#[rstest]
#[case::terminal_400_drops("EntityTooLarge", 400, true, 1, UploadOutcome::Dropped)]
#[case::terminal_403_drops("EntityTooLarge", 403, true, 1, UploadOutcome::Dropped)]
#[case::terminal_400_kept("EntityTooLarge", 400, false, 1, UploadOutcome::Retry)]
#[case::terminal_403_kept("EntityTooLarge", 403, false, 3, UploadOutcome::Retry)]
#[case::server_error_is_not_terminal("EntityTooLarge", 500, true, 3, UploadOutcome::Retry)]
#[tokio::test(start_paused = true)]
async fn only_configured_terminal_errors_are_dropped(
    #[case] code: &str,
    #[case] status: u16,
    #[case] drop_on_terminal_error: bool,
    #[case] requests: usize,
    #[case] expected: UploadOutcome,
) {
    let body = format!("<Error><Code>{code}</Code></Error>");
    let client = FakeS3::with_replies(vec![Reply::Response(status, body); 3]);
    let mut transport = transport(&client, Arc::new(static_credentials()));
    transport.drop_on_terminal_error = drop_on_terminal_error;
    let outcome = transport
        .async_upload_data_to_s3(&element("2026-01-02/id.json", ID_JSON))
        .await;
    assert_eq!(outcome, expected);
    assert_eq!(client.requests().len(), requests);
}

#[rstest]
#[case::too_many_requests(429, "", vec![true])]
#[case::unavailable(503, "", vec![true, true, true])]
#[case::slow_down(400, "<Error><Code>SlowDown</Code></Error>", vec![true])]
#[case::server_error(500, "", vec![false, false, false])]
#[case::delivered(200, "", vec![false])]
#[tokio::test(start_paused = true)]
async fn adaptive_sampling_counts_throttle_signals(
    #[case] status: u16,
    #[case] body: &str,
    #[case] expected: Vec<bool>,
) {
    let client = FakeS3::with_replies(vec![Reply::Response(status, body.to_owned()); 3]);
    let samples = Samples::default();
    let mut transport = transport(&client, Arc::new(static_credentials()));
    transport.throttle = Some(Arc::new(samples.clone()));
    transport
        .async_upload_data_to_s3(&element("2026-01-02/id.json", ID_JSON))
        .await;
    assert_eq!(samples.recorded(), expected);
}

#[tokio::test]
async fn a_failed_send_is_a_throttle_sample_and_a_retry() {
    let client = FakeS3::with_replies(vec![Reply::Failure]);
    let samples = Samples::default();
    let mut transport = transport(&client, Arc::new(static_credentials()));
    transport.throttle = Some(Arc::new(samples.clone()));
    let outcome = transport
        .async_upload_data_to_s3(&element("2026-01-02/id.json", ID_JSON))
        .await;
    assert_eq!(outcome, UploadOutcome::Retry);
    assert_eq!(samples.recorded(), vec![true]);
    assert_eq!(client.requests().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn every_attempt_resolves_the_credentials_and_signs_again() {
    let client = FakeS3::with_replies(vec![
        Reply::Response(500, String::new()),
        Reply::Response(500, String::new()),
        Reply::Response(200, String::new()),
    ]);
    let generations = Arc::new(Generations::default());
    let transport = transport(&client, Arc::clone(&generations));
    let outcome = transport
        .async_upload_data_to_s3(&element("2026-01-02/id.json", ID_JSON))
        .await;
    assert_eq!(outcome, UploadOutcome::Delivered);
    assert_eq!(generations.calls.load(Ordering::SeqCst), 3);
    let requests = client.requests();
    let scopes: Vec<&str> = requests.iter().map(credential_scope).collect();
    assert_eq!(scopes, ["AKIA1", "AKIA2", "AKIA3"]);
}

#[tokio::test]
async fn a_put_signs_one_frozen_credential_snapshot() {
    let client = FakeS3::default();
    let generations = Arc::new(Generations::default());
    let transport = transport(&client, Arc::clone(&generations));
    transport
        .async_upload_data_to_s3(&element("2026-01-02/id.json", ID_JSON))
        .await;
    assert_eq!(generations.calls.load(Ordering::SeqCst), 1);
    let requests = client.requests();
    let request = &requests[0];
    let unsigned = unsigned_put_headers(request);
    let expected = sign_request(
        &RequestToSign {
            method: "PUT",
            url: &request.url,
            headers: &unsigned,
            content_sha256: header(request, "x-amz-content-sha256").expect("a payload hash"),
            region: "us-east-1",
            now: fixed_time(),
        },
        &Credentials {
            access_key_id: SecretString::from("AKIA1"),
            secret_access_key: SecretString::from("secret1"),
            session_token: Some(SecretString::from("token1")),
        },
    )
    .expect("the request signs");
    let expected_authorization = expected
        .iter()
        .find(|(name, _)| name == "authorization")
        .map(|(_, value)| value.as_str());
    assert_eq!(
        header(request, "authorization"),
        expected_authorization,
        "a mixed-generation signature would not match the frozen snapshot"
    );
    assert_eq!(header(request, "x-amz-security-token"), Some("token1"));
}

fn unsigned_put_headers(request: &S3Request) -> Vec<(String, String)> {
    request
        .headers
        .iter()
        .filter(|(name, _)| {
            !matches!(
                name.to_ascii_lowercase().as_str(),
                "authorization" | "x-amz-date" | "x-amz-security-token"
            )
        })
        .cloned()
        .collect()
}

#[tokio::test]
async fn a_payload_without_a_body_is_serialized_as_json() {
    let client = FakeS3::default();
    let transport = transport(&client, Arc::new(static_credentials()));
    let mut log_element = element("2026-01-02/id.json", "");
    log_element.body = None;
    log_element.payload = Json::Text("payload".to_owned());
    transport.async_upload_data_to_s3(&log_element).await;
    let requests = client.requests();
    assert_eq!(requests[0].body, b"\"payload\"".to_vec());
    assert_eq!(
        header(&requests[0], "content-md5"),
        Some(STANDARD.encode(Md5::digest(b"\"payload\"")).as_str())
    );
}
