#![cfg(test)]

use std::collections::VecDeque;
use std::future::{Future, ready};
use std::iter::once;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use http::Method;
use rstest::rstest;
use secrecy::SecretString;
use sha2::{Digest as _, Sha256};

use crate::s3_transport::download::EMPTY_SHA256;
use crate::s3_transport::error::S3TransportError;
use crate::s3_transport::signing::{
    Clock, CredentialError, CredentialSource, Credentials, RequestToSign, StaticCredentials,
    sign_request,
};
use crate::s3_transport::transport::{S3HttpClient, S3Request, S3Response, S3Transport};
use crate::s3_transport::url_construction::S3Endpoint;

const GET_AUTHORIZATION: &str = "AWS4-HMAC-SHA256 Credential=AKIAEXAMPLE/20260102/us-east-1/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date;x-amz-security-token, Signature=f24fb455950541be31ae93e0497584533d63b3910397a4b7483a53810cb0b5c4";

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
    requests: Mutex<Vec<S3Request>>,
}

impl FakeS3 {
    fn with_replies(replies: Vec<Reply>) -> Self {
        let fake = Self::default();
        fake.state.replies.lock().expect("replies").extend(replies);
        fake
    }

    fn requests(&self) -> Vec<S3Request> {
        self.state.requests.lock().expect("requests").clone()
    }
}

impl S3HttpClient for FakeS3 {
    fn send(
        &self,
        request: S3Request,
    ) -> Pin<Box<dyn Future<Output = Result<S3Response, S3TransportError>> + Send + '_>> {
        Box::pin(async move {
            self.state.requests.lock().expect("requests").push(request);
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

fn fixed_time() -> SystemTime {
    let stamp =
        chrono::DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z").expect("the time parses");
    let seconds = u64::try_from(stamp.timestamp()).expect("the time is after the epoch");
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds))
        .expect("the time fits")
}

fn transport<Credential>(client: &FakeS3, credentials: Arc<Credential>) -> S3Transport
where
    Credential: CredentialSource + 'static,
{
    S3Transport {
        client: Arc::new(client.clone()),
        endpoint: S3Endpoint {
            bucket: "logs".to_owned(),
            region: "us-east-1".to_owned(),
            endpoint_url: Some("http://127.0.0.1:9000".to_owned()),
            virtual_hosted_style: false,
        },
        credentials,
        encryption: None,
        drop_on_terminal_error: true,
        throttle: None,
        clock: Arc::new(FixedClock(fixed_time())),
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

fn header<'request>(request: &'request S3Request, name: &str) -> Option<&'request str> {
    request
        .headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn object_key() -> &'static str {
    "2026-01-02/id.json"
}

#[test]
fn the_empty_sha256_is_the_hash_of_an_empty_body() {
    assert_eq!(hex::encode(Sha256::digest(b"")), EMPTY_SHA256);
}

#[tokio::test]
async fn get_returns_the_parsed_object() {
    let client = FakeS3::with_replies(vec![Reply::Response(
        200,
        "{\"id\":\"request\"}".to_owned(),
    )]);
    let transport = transport(&client, Arc::new(static_credentials()));
    let object = transport.download_object(object_key()).await;
    assert_eq!(
        object,
        Some(Json::Object(
            once(("id".to_owned(), Json::Text("request".to_owned()))).collect()
        ))
    );
    let requests = client.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, Method::GET);
    assert_eq!(request.url, "http://127.0.0.1:9000/logs/2026-01-02/id.json");
    assert!(request.body.is_empty());
    assert_eq!(header(request, "x-amz-content-sha256"), Some(EMPTY_SHA256));
    assert_eq!(header(request, "authorization"), Some(GET_AUTHORIZATION));
}

#[rstest]
#[case::not_found(404)]
#[case::server_error(500)]
#[case::forbidden(403)]
#[tokio::test]
async fn get_returns_none_for_other_statuses(#[case] status: u16) {
    let client = FakeS3::with_replies(vec![
        Reply::Response(status, "{\"id\":\"request\"}".to_owned()),
        Reply::Response(200, "{\"id\":\"request\"}".to_owned()),
    ]);
    let transport = transport(&client, Arc::new(static_credentials()));
    assert_eq!(transport.download_object(object_key()).await, None);
    assert_eq!(client.requests().len(), 1);
}

#[tokio::test]
async fn get_returns_none_for_invalid_json() {
    let client = FakeS3::with_replies(vec![Reply::Response(200, "not json".to_owned())]);
    let transport = transport(&client, Arc::new(static_credentials()));
    assert_eq!(transport.download_object(object_key()).await, None);
}

#[rstest]
#[case::list("[1,2]")]
#[case::text("\"text\"")]
#[case::number("3")]
#[case::null("null")]
#[tokio::test]
async fn get_returns_none_for_a_non_object_body(#[case] body: &str) {
    let client = FakeS3::with_replies(vec![Reply::Response(200, body.to_owned())]);
    let transport = transport(&client, Arc::new(static_credentials()));
    assert_eq!(transport.download_object(object_key()).await, None);
}

#[tokio::test]
async fn get_reports_a_transport_failure_as_none() {
    let client = FakeS3::with_replies(vec![Reply::Failure]);
    let transport = transport(&client, Arc::new(static_credentials()));
    assert_eq!(transport.download_object(object_key()).await, None);
    assert_eq!(client.requests().len(), 1);
}

#[tokio::test]
async fn get_signs_with_one_credential_resolution() {
    let client = FakeS3::with_replies(vec![Reply::Response(
        200,
        "{\"id\":\"request\"}".to_owned(),
    )]);
    let generations = Arc::new(Generations::default());
    let transport = transport(&client, Arc::clone(&generations));
    transport.download_object(object_key()).await;
    assert_eq!(
        generations.calls.load(Ordering::SeqCst),
        1,
        "LiteLLM calls get_credentials once per GET"
    );
    let requests = client.requests();
    let request = &requests[0];
    let expected = sign_request(
        &RequestToSign {
            method: "GET",
            url: &request.url,
            headers: &[("x-amz-content-sha256".to_owned(), EMPTY_SHA256.to_owned())],
            content_sha256: EMPTY_SHA256,
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
    assert_eq!(header(request, "authorization"), expected_authorization);
    assert_eq!(header(request, "x-amz-security-token"), Some("token1"));
}
