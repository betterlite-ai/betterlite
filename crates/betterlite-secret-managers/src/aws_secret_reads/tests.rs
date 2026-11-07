#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::future::ready;
use std::io;
use std::path::Path;
use std::str::from_utf8;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_credential_types::Credentials;
use http::StatusCode;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};
use serde_json::error::Category;
use thiserror::Error;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::{MakeWriter, fmt};

use super::{AwsBlockingTransport, AwsReadError, SecretField, parse_primary_secret};
use crate::aws::request_signing::{
    ActionRequest, AwsCredentialsInfo, AwsOptionalParams, AwsRequestError, AwsTransport,
    CredentialResolver, NoBody, TransportError, TransportFuture, TransportRequest,
    TransportResponse,
};
use crate::aws::{AwsDependencies, AwsSecretsManager};

const ACCESS_KEY: &str = "AKIDEXAMPLE";
const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";

#[derive(Debug, Error)]
enum ResolverFailure {
    #[error("credentials are unavailable")]
    Unavailable,
}

struct FakeResolver {
    fail: bool,
    seen: Mutex<Vec<Option<String>>>,
}

impl FakeResolver {
    fn working() -> Self {
        Self {
            fail: false,
            seen: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            fail: true,
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl CredentialResolver for FakeResolver {
    type Error = ResolverFailure;

    fn resolve_credentials(
        &self,
        optional_params: &mut AwsOptionalParams,
    ) -> Result<AwsCredentialsInfo, ResolverFailure> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(optional_params.aws_region_name.clone());
        if self.fail {
            return Err(ResolverFailure::Unavailable);
        }
        Ok(AwsCredentialsInfo {
            aws_region_name: optional_params
                .aws_region_name
                .clone()
                .unwrap_or_else(|| "us-east-1".to_owned()),
            aws_bedrock_runtime_endpoint: None,
            credentials: Credentials::new(ACCESS_KEY, SECRET_KEY, None, None, "test"),
        })
    }
}

type Reply = Result<TransportResponse, TransportError<io::Error>>;

struct FakeTransport {
    replies: Mutex<VecDeque<Reply>>,
    sent: Mutex<Vec<(String, String)>>,
    targets: Mutex<Vec<Option<String>>>,
    posts: AtomicUsize,
    blocking_posts: AtomicUsize,
}

impl FakeTransport {
    fn with_replies(replies: Vec<Reply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(VecDeque::from(replies)),
            sent: Mutex::new(Vec::new()),
            targets: Mutex::new(Vec::new()),
            posts: AtomicUsize::new(0),
            blocking_posts: AtomicUsize::new(0),
        })
    }

    fn next(&self) -> Reply {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_else(|| answer(500, b"no reply left"))
    }

    fn record(&self, request: &TransportRequest<'_>) {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((
                request.url.to_owned(),
                String::from_utf8(request.body.to_vec()).unwrap(),
            ));
        self.targets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(
                request
                    .headers
                    .get("x-amz-target")
                    .and_then(|value| value.to_str().ok())
                    .map(str::to_owned),
            );
    }

    fn sent(&self) -> Vec<(String, String)> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn targets(&self) -> Vec<Option<String>> {
        self.targets
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl AwsTransport for FakeTransport {
    type Error = io::Error;

    fn post<'request>(
        &'request self,
        request: TransportRequest<'request>,
    ) -> TransportFuture<'request, io::Error> {
        self.posts.fetch_add(1, Ordering::Relaxed);
        self.record(&request);
        Box::pin(ready(self.next()))
    }
}

impl AwsBlockingTransport for FakeTransport {
    fn post_blocking(&self, request: TransportRequest<'_>) -> Reply {
        self.blocking_posts.fetch_add(1, Ordering::Relaxed);
        self.record(&request);
        self.next()
    }
}

#[derive(Clone, Copy, Debug)]
enum PostReply {
    Text,
    MissingKey,
    NonTextValue,
    NotJson,
    ErrorStatus,
    NonUtf8ErrorStatus,
    Timeout,
    FailedTransport,
    TooLarge,
    Credentials,
}

fn oversized_body() -> Vec<u8> {
    let mut body = br#"{"SecretString":""#.to_vec();
    body.resize(0x0010_0001, b'a');
    body.extend_from_slice(br#""}"#);
    body
}

fn post_reply(reply: PostReply) -> Reply {
    match reply {
        PostReply::Text => answer(200, br#"{"SecretString":"hunter2"}"#),
        PostReply::MissingKey => answer(200, br#"{"SecretBinary":"aHVudGVyMg=="}"#),
        PostReply::NonTextValue => answer(200, br#"{"SecretString":7}"#),
        PostReply::NotJson => answer(200, b"plain text"),
        PostReply::ErrorStatus => answer(500, b"internal failure"),
        PostReply::NonUtf8ErrorStatus => Ok(TransportResponse {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            body: vec![0xff, 0xfe],
        }),
        PostReply::Timeout => Err(TransportError::Timeout),
        PostReply::FailedTransport => Err(TransportError::Failed(io::Error::from(
            io::ErrorKind::ConnectionRefused,
        ))),
        PostReply::TooLarge => answer(200, &oversized_body()),
        PostReply::Credentials => answer(200, b"{}"),
    }
}

fn answer(status: u16, body: &[u8]) -> Reply {
    let code = StatusCode::from_u16(status)
        .map_err(|error| TransportError::Failed(io::Error::other(error)))?;
    Ok(TransportResponse {
        status: code,
        body: body.to_vec(),
    })
}

fn dns_suffix(_region: &str) -> &'static str {
    "amazonaws.com"
}

fn signing_time() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_709_641_845))
        .unwrap()
}

fn manager(
    transport: &Arc<FakeTransport>,
    resolver: FakeResolver,
    environment: BTreeMap<String, String>,
) -> AwsSecretsManager<FakeResolver, FakeTransport> {
    AwsSecretsManager {
        aws_region_name: None,
        aws_role_name: None,
        aws_session_name: None,
        aws_external_id: None,
        aws_profile_name: None,
        aws_web_identity_token: None,
        aws_sts_endpoint: None,
        replica_regions: Vec::new(),
        kms_key_id: None,
        dependencies: AwsDependencies {
            environment,
            transport: Arc::clone(transport),
            resolver,
            dns_suffix,
            clock: signing_time,
        },
    }
}

fn region_params(region: &str) -> AwsOptionalParams {
    AwsOptionalParams {
        aws_role_name: None,
        aws_session_name: None,
        aws_region_name: Some(region.to_owned()),
        aws_external_id: None,
        aws_profile_name: None,
        aws_web_identity_token: None,
        aws_sts_endpoint: None,
        aws_access_key_id: None,
        aws_secret_access_key: None,
        aws_session_token: None,
        aws_bedrock_runtime_endpoint: None,
    }
}

fn describe(
    reply: Result<BTreeMap<String, SecretField>, AwsRequestError<ResolverFailure, io::Error>>,
) -> Result<String, String> {
    reply
        .map(|fields| {
            fields
                .into_iter()
                .map(|(name, field)| match field {
                    SecretField::Text(text) => format!("{name}={text}"),
                    SecretField::Other(_) => format!("{name}=other"),
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .map_err(|error| error.to_string())
}

fn bootstrap_environment() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("AWS_ACCESS_KEY_ID".to_owned(), "env-key".to_owned()),
        ("AWS_SECRET_ACCESS_KEY".to_owned(), "env-secret".to_owned()),
        ("AWS_REGION".to_owned(), "eu-central-2".to_owned()),
        ("AWS_REGION_NAME".to_owned(), "us-east-1".to_owned()),
    ])
}

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .unwrap()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

fn capture_logs() -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}

#[tokio::test]
async fn direct_reads_return_the_secret_string() {
    let transport = FakeTransport::with_replies(vec![
        answer(200, br#"{"SecretString":"hunter2"}"#),
        answer(200, br#"{"SecretString":"hunter2"}"#),
    ]);
    let manager = manager(&transport, FakeResolver::working(), BTreeMap::new());
    let async_value = manager
        .async_read_secret("my/secret", None, None, None)
        .await
        .unwrap();
    let sync_value = manager
        .sync_read_secret("my/secret", None, None, None)
        .unwrap();
    assert_eq!(async_value.unwrap().expose_secret(), "hunter2");
    assert_eq!(sync_value.unwrap().expose_secret(), "hunter2");
    let sent = transport.sent();
    assert_eq!(
        sent[0],
        (
            "https://secretsmanager.us-east-1.amazonaws.com".to_owned(),
            r#"{"SecretId":"my/secret"}"#.to_owned()
        )
    );
    assert_eq!(sent[1], sent[0]);
    assert_eq!(
        transport.targets(),
        [
            Some("secretsmanager.GetSecretValue".to_owned()),
            Some("secretsmanager.GetSecretValue".to_owned())
        ]
    );
}

#[tokio::test]
async fn optional_params_reach_the_direct_request() {
    let transport = FakeTransport::with_replies(vec![
        answer(200, br#"{"SecretString":"hunter2"}"#),
        answer(200, br#"{"SecretString":"hunter2"}"#),
    ]);
    let manager = manager(&transport, FakeResolver::working(), BTreeMap::new());
    let mut async_params = region_params("eu-west-1");
    let async_value = manager
        .async_read_secret("my/secret", Some(&mut async_params), None, None)
        .await
        .unwrap();
    let mut sync_params = region_params("eu-west-1");
    let sync_value = manager
        .sync_read_secret("my/secret", Some(&mut sync_params), None, None)
        .unwrap();
    assert_eq!(async_value.unwrap().expose_secret(), "hunter2");
    assert_eq!(sync_value.unwrap().expose_secret(), "hunter2");
    let sent = transport.sent();
    assert_eq!(sent[0].0, "https://secretsmanager.eu-west-1.amazonaws.com");
    assert_eq!(sent[1].0, "https://secretsmanager.eu-west-1.amazonaws.com");
}

#[tokio::test]
async fn sync_reads_return_bootstrap_names_from_the_environment() {
    let mut environment = bootstrap_environment();
    environment.insert(
        "AWS_BEDROCK_RUNTIME_ENDPOINT".to_owned(),
        "https://bedrock-runtime.us-east-1.amazonaws.com".to_owned(),
    );
    let transport = FakeTransport::with_replies(Vec::new());
    let configured = manager(&transport, FakeResolver::working(), environment);
    let access_key = configured
        .sync_read_secret("AWS_ACCESS_KEY_ID", None, None, Some("gateway"))
        .unwrap();
    let secret_key = configured
        .sync_read_secret("AWS_SECRET_ACCESS_KEY", None, None, None)
        .unwrap();
    let region_name = configured
        .sync_read_secret("AWS_REGION_NAME", None, None, None)
        .unwrap();
    let region = configured
        .sync_read_secret("AWS_REGION", None, None, None)
        .unwrap();
    let endpoint = configured
        .sync_read_secret("AWS_BEDROCK_RUNTIME_ENDPOINT", None, None, None)
        .unwrap();
    assert_eq!(access_key.unwrap().expose_secret(), "env-key");
    assert_eq!(secret_key.unwrap().expose_secret(), "env-secret");
    assert_eq!(region_name.unwrap().expose_secret(), "us-east-1");
    assert_eq!(region.unwrap().expose_secret(), "eu-central-2");
    assert_eq!(
        endpoint.unwrap().expose_secret(),
        "https://bedrock-runtime.us-east-1.amazonaws.com"
    );
    assert_eq!(transport.posts.load(Ordering::Relaxed), 0);
    assert_eq!(transport.blocking_posts.load(Ordering::Relaxed), 0);
    let empty = FakeTransport::with_replies(vec![answer(200, br#"{"SecretString":"from-aws"}"#)]);
    let unset = manager(&empty, FakeResolver::working(), BTreeMap::new());
    assert!(
        unset
            .sync_read_secret("AWS_REGION", None, None, None)
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn async_reads_post_bootstrap_names_to_aws() {
    let transport =
        FakeTransport::with_replies(vec![answer(200, br#"{"SecretString":"from-aws"}"#)]);
    let manager = manager(&transport, FakeResolver::working(), bootstrap_environment());
    let value = manager
        .async_read_secret("AWS_REGION_NAME", None, None, None)
        .await
        .unwrap();
    assert_eq!(value.unwrap().expose_secret(), "from-aws");
    assert_eq!(transport.posts.load(Ordering::Relaxed), 1);
    assert_eq!(transport.blocking_posts.load(Ordering::Relaxed), 0);
    assert_eq!(
        transport.sent()[0],
        (
            "https://secretsmanager.us-east-1.amazonaws.com".to_owned(),
            r#"{"SecretId":"AWS_REGION_NAME"}"#.to_owned()
        )
    );
}

#[tokio::test]
async fn primary_reads_return_the_requested_key() {
    let transport = FakeTransport::with_replies(vec![
        answer(200, br#"{"db":"pw-one","api":"pw-two"}"#),
        answer(200, br#"{"db":"pw-one","api":"pw-two"}"#),
        answer(200, br#"{"db":"pw-one","api":"pw-two"}"#),
        answer(200, br#"{"db":"pw-one","api":"pw-two"}"#),
    ]);
    let manager = manager(&transport, FakeResolver::working(), BTreeMap::new());
    let async_value = manager
        .async_read_secret("api", None, None, Some("gateway"))
        .await
        .unwrap();
    let sync_value = manager
        .sync_read_secret("api", None, None, Some("gateway"))
        .unwrap();
    let missing = manager
        .async_read_from_primary("missing", "gateway")
        .await
        .unwrap();
    let missing_sync = manager
        .sync_read_from_primary("missing", "gateway")
        .unwrap();
    assert_eq!(async_value.unwrap().expose_secret(), "pw-two");
    assert_eq!(sync_value.unwrap().expose_secret(), "pw-two");
    assert!(missing.is_none());
    assert!(missing_sync.is_none());
    assert_eq!(
        transport.sent()[0],
        (
            "https://secretsmanager.us-east-1.amazonaws.com".to_owned(),
            r#"{"SecretId":"gateway"}"#.to_owned()
        )
    );
}

#[tokio::test]
async fn primary_reads_have_no_optional_params() {
    let transport = FakeTransport::with_replies(vec![
        answer(200, br#"{"db":"pw"}"#),
        answer(200, br#"{"db":"pw"}"#),
    ]);
    let manager = manager(&transport, FakeResolver::working(), BTreeMap::new());
    let mut async_params = region_params("eu-west-1");
    let async_value = manager
        .async_read_secret("db", Some(&mut async_params), None, Some("gateway"))
        .await
        .unwrap();
    let mut sync_params = region_params("eu-west-1");
    let sync_value = manager
        .sync_read_secret("db", Some(&mut sync_params), None, Some("gateway"))
        .unwrap();
    assert_eq!(async_value.unwrap().expose_secret(), "pw");
    assert_eq!(sync_value.unwrap().expose_secret(), "pw");
    let sent = transport.sent();
    assert_eq!(sent[0].0, "https://secretsmanager.us-east-1.amazonaws.com");
    assert_eq!(sent[0].1, r#"{"SecretId":"gateway"}"#);
    assert_eq!(sent[1], sent[0]);
}

#[rstest]
#[case::failed_primary(500, "internal failure")]
#[case::empty_primary_text(200, r#"{"SecretString":""}"#)]
#[case::missing_primary_text(200, "{}")]
#[tokio::test]
async fn a_missing_or_failed_primary_reads_as_none(#[case] status: u16, #[case] body: &str) {
    let transport = FakeTransport::with_replies(vec![
        answer(status, body.as_bytes()),
        answer(status, body.as_bytes()),
    ]);
    let manager = manager(&transport, FakeResolver::working(), BTreeMap::new());
    let async_value = manager
        .async_read_secret("db", None, None, Some("gateway"))
        .await
        .unwrap();
    let sync_value = manager
        .sync_read_secret("db", None, None, Some("gateway"))
        .unwrap();
    assert!(async_value.is_none());
    assert!(sync_value.is_none());
}

#[rstest]
#[case::bare_text(r#"{"SecretString":"not json"}"#)]
#[case::json_string(r#"{"SecretString":"\"sk-primary-value\""}"#)]
#[case::json_array(r#"{"SecretString":"[1,2]"}"#)]
#[case::json_null(r#"{"SecretString":"null"}"#)]
#[tokio::test]
async fn an_invalid_primary_raises(#[case] reply: &str) {
    let transport = FakeTransport::with_replies(vec![
        answer(200, reply.as_bytes()),
        answer(200, reply.as_bytes()),
    ]);
    let manager = manager(&transport, FakeResolver::working(), BTreeMap::new());
    let async_error = manager
        .async_read_secret("db", None, None, Some("gateway"))
        .await
        .unwrap_err();
    let sync_error = manager
        .sync_read_secret("db", None, None, Some("gateway"))
        .unwrap_err();
    assert!(matches!(async_error, AwsReadError::Primary(_)));
    assert!(matches!(sync_error, AwsReadError::Primary(_)));
}

#[tokio::test]
async fn an_empty_primary_name_reads_directly() {
    let transport = FakeTransport::with_replies(vec![
        answer(200, br#"{"SecretString":"direct"}"#),
        answer(200, br#"{"SecretString":"direct"}"#),
    ]);
    let manager = manager(&transport, FakeResolver::working(), BTreeMap::new());
    let async_value = manager
        .async_read_secret("db", None, None, Some(""))
        .await
        .unwrap();
    let sync_value = manager
        .sync_read_secret("db", None, None, Some(""))
        .unwrap();
    assert_eq!(async_value.unwrap().expose_secret(), "direct");
    assert_eq!(sync_value.unwrap().expose_secret(), "direct");
    let sent = transport.sent();
    assert_eq!(sent[0].1, r#"{"SecretId":"db"}"#);
    assert_eq!(sent[1].1, r#"{"SecretId":"db"}"#);
}

#[rstest]
#[case::absent(None, 0)]
#[case::empty(Some(""), 0)]
#[case::empty_object(Some("{}"), 0)]
#[case::text_values(Some(r#"{"a":"one","b":"two"}"#), 2)]
#[case::mixed_values(Some(r#"{"a":"one","b":2,"c":[1],"d":null,"e":true}"#), 1)]
fn primary_maps_keep_only_text_values(#[case] text: Option<&str>, #[case] count: usize) {
    let secret = text.map(SecretString::from);
    let fields = parse_primary_secret(secret.as_ref()).unwrap();
    assert_eq!(fields.len(), count);
}

#[test]
fn duplicate_primary_keys_resolve_to_the_last() {
    let secret = SecretString::from(r#"{"a":"one","a":"two"}"#);
    let fields = parse_primary_secret(Some(&secret)).unwrap();
    assert_eq!(
        fields.get("a").map(ExposeSecret::expose_secret),
        Some("two")
    );
}

#[test]
fn primary_error_hides_the_value() {
    let secret = SecretString::from("\"sk-primary-value\"");
    let error = parse_primary_secret(Some(&secret)).unwrap_err();
    assert_eq!(error.category, Category::Data);
    assert!(
        error
            .to_string()
            .starts_with("the primary secret is not a JSON object")
    );
    assert!(!error.to_string().contains("sk-primary-value"));
    assert!(!format!("{error:?}").contains("sk-primary-value"));
}

#[tokio::test]
async fn read_failure_log_hides_the_reply() {
    let (logs, _guard) = capture_logs();
    let transport = FakeTransport::with_replies(vec![answer(200, br#""sk-reply-value""#)]);
    let manager = manager(&transport, FakeResolver::working(), BTreeMap::new());
    let value = manager
        .async_read_secret("my/secret", None, None, None)
        .await
        .unwrap();
    assert!(value.is_none());
    let text = logs.text();
    assert!(text.contains("ERROR failed to read the secret"));
    assert!(text.contains("my/secret"));
    assert!(!text.contains("sk-reply-value"));
    assert_eq!(text.lines().filter(|line| !line.is_empty()).count(), 1);
}

#[rstest]
#[case::timeout(PostReply::Timeout)]
#[case::failed_transport(PostReply::FailedTransport)]
#[case::oversized_reply(PostReply::TooLarge)]
#[case::error_status(PostReply::ErrorStatus)]
#[case::non_utf8_error_status(PostReply::NonUtf8ErrorStatus)]
#[case::not_json(PostReply::NotJson)]
#[case::missing_secret_string(PostReply::MissingKey)]
#[case::non_text_secret_string(PostReply::NonTextValue)]
#[tokio::test]
async fn request_failure_reads_as_none(#[case] reply: PostReply) {
    let transport = FakeTransport::with_replies(vec![post_reply(reply), post_reply(reply)]);
    let manager = manager(&transport, FakeResolver::working(), BTreeMap::new());
    let async_value = manager
        .async_read_secret("my/secret", None, None, None)
        .await
        .unwrap();
    let sync_value = manager
        .sync_read_secret("my/secret", None, None, None)
        .unwrap();
    assert!(async_value.is_none());
    assert!(sync_value.is_none());
    assert_eq!(transport.posts.load(Ordering::Relaxed), 1);
    assert_eq!(transport.blocking_posts.load(Ordering::Relaxed), 1);
}

#[rstest]
#[case::timeout(PostReply::Timeout, "timeout")]
#[case::failed_transport(PostReply::FailedTransport, "transport")]
#[case::oversized_reply(PostReply::TooLarge, "response_too_large")]
#[case::error_status(PostReply::ErrorStatus, "http")]
#[case::non_utf8_error_status(PostReply::NonUtf8ErrorStatus, "error_text")]
#[case::not_json(PostReply::NotJson, "response")]
#[case::missing_secret_string(PostReply::MissingKey, "missing_secret_string")]
#[case::non_text_secret_string(PostReply::NonTextValue, "missing_secret_string")]
#[tokio::test]
async fn failed_reads_log_the_kind_of_failure(#[case] reply: PostReply, #[case] failure: &str) {
    let (logs, _guard) = capture_logs();
    let transport = FakeTransport::with_replies(vec![post_reply(reply)]);
    let manager = manager(&transport, FakeResolver::working(), BTreeMap::new());
    let value = manager
        .async_read_secret("my/secret", None, None, None)
        .await
        .unwrap();
    assert!(value.is_none());
    let text = logs.text();
    assert_eq!(text.lines().filter(|line| !line.is_empty()).count(), 1);
    assert!(text.contains("ERROR failed to read the secret"));
    assert!(text.contains(&format!("failure=\"{failure}\"")));
}

#[tokio::test]
async fn credential_failure_reaches_the_caller() {
    let transport = FakeTransport::with_replies(Vec::new());
    let manager = manager(&transport, FakeResolver::failing(), BTreeMap::new());
    let async_error = manager
        .async_read_secret("my/secret", None, None, None)
        .await
        .unwrap_err();
    let sync_error = manager
        .sync_read_secret("my/secret", None, None, None)
        .unwrap_err();
    assert!(matches!(
        async_error,
        AwsReadError::Request(AwsRequestError::Credentials(ResolverFailure::Unavailable))
    ));
    assert!(matches!(
        sync_error,
        AwsReadError::Request(AwsRequestError::Credentials(ResolverFailure::Unavailable))
    ));
    assert_eq!(transport.posts.load(Ordering::Relaxed), 0);
    assert_eq!(transport.blocking_posts.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn each_path_uses_its_own_post() {
    let transport = FakeTransport::with_replies(vec![
        answer(200, br#"{"SecretString":"async"}"#),
        answer(200, br#"{"SecretString":"sync"}"#),
    ]);
    let manager = manager(&transport, FakeResolver::working(), BTreeMap::new());
    manager
        .async_read_secret("first", None, None, None)
        .await
        .unwrap();
    assert_eq!(transport.posts.load(Ordering::Relaxed), 1);
    assert_eq!(transport.blocking_posts.load(Ordering::Relaxed), 0);
    manager
        .sync_read_secret("second", None, None, None)
        .unwrap();
    assert_eq!(transport.posts.load(Ordering::Relaxed), 1);
    assert_eq!(transport.blocking_posts.load(Ordering::Relaxed), 1);
}

#[rstest]
#[case::text_reply(PostReply::Text)]
#[case::missing_key(PostReply::MissingKey)]
#[case::not_json(PostReply::NotJson)]
#[case::error_status(PostReply::ErrorStatus)]
#[case::non_utf8_error_status(PostReply::NonUtf8ErrorStatus)]
#[case::timeout(PostReply::Timeout)]
#[case::failed_transport(PostReply::FailedTransport)]
#[case::oversized_reply(PostReply::TooLarge)]
#[case::credential_failure(PostReply::Credentials)]
#[tokio::test]
async fn blocking_post_matches_async_post(#[case] reply: PostReply) {
    let request: ActionRequest<'_, NoBody> = ActionRequest {
        action: "GetSecretValue",
        secret_name: "my/secret",
        request_data: None,
    };
    let async_transport = FakeTransport::with_replies(vec![post_reply(reply)]);
    let async_manager = manager(&async_transport, resolver_for(reply), BTreeMap::new());
    let async_reply = async_manager
        .async_post_action::<NoBody, BTreeMap<String, SecretField>>(&request, None, None)
        .await;
    let blocking_transport = FakeTransport::with_replies(vec![post_reply(reply)]);
    let blocking_manager = manager(&blocking_transport, resolver_for(reply), BTreeMap::new());
    let blocking_reply = blocking_manager
        .blocking_post_action::<NoBody, BTreeMap<String, SecretField>>(&request, None, None);
    assert_eq!(describe(async_reply), describe(blocking_reply));
    assert_eq!(async_transport.posts.load(Ordering::Relaxed), 1);
    assert_eq!(blocking_transport.blocking_posts.load(Ordering::Relaxed), 1);
}

fn resolver_for(reply: PostReply) -> FakeResolver {
    if matches!(reply, PostReply::Credentials) {
        FakeResolver::failing()
    } else {
        FakeResolver::working()
    }
}

#[test]
fn replays_the_aws_primary_secret_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/aws_primary_secret");
    let parsed = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            from_utf8(&bytes).map(|text| parse_primary_secret(Some(&SecretString::from(text))))
        })
        .collect::<Vec<_>>();
    assert!(parsed.len() >= 10);
    assert!(parsed.iter().all(Result::is_ok));
}
