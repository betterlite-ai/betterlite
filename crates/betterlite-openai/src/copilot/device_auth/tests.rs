#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::future::{Future, ready};
use std::io;
use std::iter;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::copilot_device::Settings;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::runtime::Builder as RuntimeBuilder;
use tracing::subscriber::with_default;
use uuid::Uuid;

use super::decode::{AccessReply, DeviceReply, REPLY_LIMIT, ReplyError};
use super::files::{FILE_LIMIT, FileAccess, LocalFiles};
use super::output::{OperatorOutput, StreamOutput};
use super::sleeper::{Sleeper, TokioSleeper};
use super::transport::{HttpTransport, Post, Reply, Transport, TransportFailure};
use super::{
    AccessTokenFailure, Authenticator, DeviceAuthError, DeviceCodeFailure, github_headers,
};
use crate::copilot::headers::{
    API_VERSION, COPILOT_VERSION, DEFAULT_COPILOT_API_BASE, EDITOR_PLUGIN_VERSION, USER_AGENT,
    copilot_default_headers,
};
use crate::openai_base_config::Headers;

const DEVICE_URL: &str = "https://device.example/code";
const ACCESS_URL: &str = "https://token.example/access";
const API_KEY_URL: &str = "https://key.example/key";

// serde_json::json! uses Into::into for object members, which clippy.toml disallows
fn object<const N: usize>(pairs: [(&str, Value); N]) -> Value {
    Value::Object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn text(value: &str) -> Value {
    Value::String(value.to_owned())
}

fn settings() -> Settings {
    Settings {
        token_dir: String::from("/tokens"),
        access_token_file: String::from("access-token"),
        api_key_file: String::from("api-key.json"),
        client_id: String::from("Iv1.test"),
        device_code_url: String::from(DEVICE_URL),
        access_token_url: String::from(ACCESS_URL),
        api_key_url: String::from(API_KEY_URL),
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Fault {
    #[default]
    None,
    Directory,
    Read,
    Write,
}

#[derive(Debug, Default)]
struct Files {
    content: Option<Vec<u8>>,
    fault: Fault,
    writes: Vec<Vec<u8>>,
    dirs: Vec<PathBuf>,
}

impl FileAccess for Files {
    fn ensure_dir(&mut self, path: &Path) -> io::Result<()> {
        self.dirs.push(path.to_path_buf());
        if self.fault == Fault::Directory {
            return Err(io::Error::from(io::ErrorKind::PermissionDenied));
        }
        Ok(())
    }

    fn read(&mut self, _path: &Path) -> io::Result<Vec<u8>> {
        if self.fault == Fault::Read {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        }
        self.content
            .clone()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    fn write(&mut self, _path: &Path, content: &[u8]) -> io::Result<()> {
        self.writes.push(content.to_vec());
        if self.fault == Fault::Write {
            return Err(io::Error::other("scripted write failure"));
        }
        Ok(())
    }
}

#[derive(Debug)]
struct RecordedRequest {
    url: String,
    headers: Headers,
    body: Value,
}

#[derive(Debug)]
struct Scripted {
    device_url: String,
    access_url: String,
    device_replies: VecDeque<Result<Reply, TransportFailure>>,
    access_replies: VecDeque<Result<Reply, TransportFailure>>,
    requests: Vec<RecordedRequest>,
}

impl Transport for Scripted {
    fn post_json<'call>(
        &'call mut self,
        post: Post<'call>,
    ) -> impl Future<Output = Result<Reply, TransportFailure>> + Send + 'call {
        let reply = if post.url == self.device_url.as_str() {
            self.device_replies.pop_front()
        } else if post.url == self.access_url.as_str() {
            self.access_replies.pop_front()
        } else {
            None
        };
        let headers = post.headers;
        self.requests.push(RecordedRequest {
            url: post.url.to_owned(),
            headers,
            body: serde_json::from_slice(&post.body).unwrap_or(Value::Null),
        });
        ready(reply.unwrap_or(Err(TransportFailure::Unavailable)))
    }
}

#[derive(Debug, Default)]
struct ScriptSleeper {
    sleeps: Vec<Duration>,
}

impl Sleeper for ScriptSleeper {
    fn sleep(&mut self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.sleeps.push(duration);
        ready(())
    }
}

#[derive(Debug, Default)]
struct ScriptOutput {
    messages: Vec<String>,
}

impl OperatorOutput for ScriptOutput {
    fn instructions(&mut self, message: &str) {
        self.messages.push(message.to_owned());
    }
}

fn scripted(
    device: Vec<Result<Reply, TransportFailure>>,
    access: Vec<Result<Reply, TransportFailure>>,
) -> Scripted {
    Scripted {
        device_url: String::from(DEVICE_URL),
        access_url: String::from(ACCESS_URL),
        device_replies: VecDeque::from(device),
        access_replies: VecDeque::from(access),
        requests: Vec::new(),
    }
}

type Subject = Authenticator<Files, ScriptSleeper, Scripted, ScriptOutput>;

fn authenticator(
    files: Files,
    device: Vec<Result<Reply, TransportFailure>>,
    access: Vec<Result<Reply, TransportFailure>>,
) -> Subject {
    Authenticator::new(
        settings(),
        files,
        ScriptSleeper::default(),
        scripted(device, access),
        ScriptOutput::default(),
    )
    .unwrap()
}

fn reply(status: u16, body: &Value) -> Reply {
    Reply {
        status,
        body: serde_json::to_vec(body).unwrap(),
    }
}

fn raw_reply(status: u16, body: &[u8]) -> Reply {
    Reply {
        status,
        body: body.to_vec(),
    }
}

fn device_reply() -> Reply {
    reply(
        200,
        &object([
            ("device_code", text("dev-1")),
            ("user_code", text("ABCD-1234")),
            ("verification_uri", text("https://github.com/login/device")),
        ]),
    )
}

fn token_reply(token: &str) -> Reply {
    reply(200, &object([("access_token", text(token))]))
}

fn pending_reply() -> Reply {
    reply(200, &object([("error", text("authorization_pending"))]))
}

fn logged<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_writer(log.reopen().unwrap())
        .finish();
    let result = with_default(subscriber, action);
    (result, fs::read_to_string(log.path()).unwrap())
}

fn block_on<T, F>(future: F) -> T
where
    F: Future<Output = T>,
{
    RuntimeBuilder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

#[tokio::test]
async fn a_cached_token_is_trimmed_and_skips_login() {
    let files = Files {
        content: Some(b"  cached-token \n".to_vec()),
        fault: Fault::None,
        writes: Vec::new(),
        dirs: Vec::new(),
    };
    let mut subject = authenticator(files, Vec::new(), Vec::new());

    let token = subject.get_access_token().await.unwrap();

    assert_eq!(token.expose_secret(), "cached-token");
    assert!(subject.transport.requests.is_empty());
    assert!(subject.sleeper.sleeps.is_empty());
    assert!(subject.files.writes.is_empty());
}

#[tokio::test]
async fn a_whitespace_only_cached_token_starts_login() {
    let files = Files {
        content: Some(b" \n\t ".to_vec()),
        fault: Fault::None,
        writes: Vec::new(),
        dirs: Vec::new(),
    };
    let mut subject = authenticator(
        files,
        vec![Ok(device_reply())],
        vec![Ok(token_reply("fresh"))],
    );

    let token = subject.get_access_token().await.unwrap();

    assert_eq!(token.expose_secret(), "fresh");
    assert_eq!(subject.files.writes, vec![b"fresh".to_vec()]);
    assert_eq!(subject.transport.requests.len(), 2);
}

#[tokio::test]
async fn an_unreadable_cached_token_starts_login() {
    let files = Files {
        content: None,
        fault: Fault::Read,
        writes: Vec::new(),
        dirs: Vec::new(),
    };
    let mut subject = authenticator(
        files,
        vec![Ok(device_reply())],
        vec![Ok(token_reply("fresh"))],
    );

    let token = subject.get_access_token().await.unwrap();

    assert_eq!(token.expose_secret(), "fresh");
    assert_eq!(subject.transport.requests.len(), 2);
}

#[tokio::test]
async fn a_non_utf8_cached_token_starts_login() {
    let files = Files {
        content: Some(vec![0xff, 0xfe]),
        fault: Fault::None,
        writes: Vec::new(),
        dirs: Vec::new(),
    };
    let mut subject = authenticator(
        files,
        vec![Ok(device_reply())],
        vec![Ok(token_reply("fresh"))],
    );

    let token = subject.get_access_token().await.unwrap();

    assert_eq!(token.expose_secret(), "fresh");
    assert_eq!(subject.transport.requests.len(), 2);
}

#[tokio::test]
async fn a_pending_poll_waits_five_seconds_before_succeeding() {
    let mut subject = authenticator(
        Files::default(),
        vec![Ok(device_reply())],
        vec![Ok(pending_reply()), Ok(token_reply("first"))],
    );

    let token = subject.get_access_token().await.unwrap();

    assert_eq!(token.expose_secret(), "first");
    assert_eq!(subject.sleeper.sleeps, vec![Duration::from_secs(5)]);
    assert_eq!(
        subject.output.messages,
        vec![String::from(
            "Please visit https://github.com/login/device and enter code ABCD-1234 to authenticate."
        )]
    );
    assert_eq!(subject.files.writes, vec![b"first".to_vec()]);
    assert_eq!(subject.transport.requests.len(), 2);
    assert_eq!(subject.transport.requests[0].url, DEVICE_URL);
    assert_eq!(
        subject.transport.requests[0].body,
        object([
            ("client_id", text("Iv1.test")),
            ("scope", text("read:user"))
        ])
    );
    assert_eq!(subject.transport.requests[0].headers, github_headers(None));
    assert_eq!(subject.transport.requests[1].url, ACCESS_URL);
    assert_eq!(
        subject.transport.requests[1].body,
        object([
            ("client_id", text("Iv1.test")),
            ("device_code", text("dev-1")),
            (
                "grant_type",
                text("urn:ietf:params:oauth:grant-type:device_code")
            ),
        ])
    );
    assert_eq!(subject.transport.requests[1].headers, github_headers(None));
}

#[tokio::test]
async fn twelve_pending_replies_exhaust_the_poll() {
    let pending: Vec<Result<Reply, TransportFailure>> =
        iter::repeat_with(|| Ok(pending_reply())).take(12).collect();
    let mut subject = authenticator(Files::default(), vec![Ok(device_reply())], pending);

    let error = subject.get_access_token().await.unwrap_err();

    assert!(matches!(
        &error,
        DeviceAuthError::AccessToken(AccessTokenFailure::Timeout)
    ));
    assert_eq!(error.status(), Some(400));
    assert_eq!(subject.transport.requests.len(), 13);
    assert_eq!(subject.sleeper.sleeps.len(), 12);
    assert!(subject.files.writes.is_empty());
}

#[rstest]
#[case::device_code(
    object([("user_code", text("U")), ("verification_uri", text("V"))]),
    "device_code"
)]
#[case::user_code(
    object([("device_code", text("D")), ("verification_uri", text("V"))]),
    "user_code"
)]
#[case::verification_uri(
    object([("device_code", text("D")), ("user_code", text("U"))]),
    "verification_uri"
)]
#[tokio::test]
async fn login_reports_each_missing_device_field(#[case] body: Value, #[case] name: &'static str) {
    let mut subject = authenticator(Files::default(), vec![Ok(reply(200, &body))], Vec::new());

    let error = subject.login().await.unwrap_err();

    assert!(matches!(
        &error,
        DeviceAuthError::DeviceCode(DeviceCodeFailure::MissingField { name: found }) if *found == name
    ));
    assert_eq!(error.status(), Some(400));
    assert!(subject.output.messages.is_empty());
    assert_eq!(subject.transport.requests.len(), 1);
}

#[tokio::test]
async fn three_failed_logins_end_in_a_401() {
    let missing = || Ok(reply(200, &object([("device_code", text("D"))])));
    let mut subject = authenticator(
        Files::default(),
        vec![missing(), missing(), missing()],
        Vec::new(),
    );

    let error = subject.get_access_token().await.unwrap_err();

    assert!(matches!(&error, DeviceAuthError::Exhausted));
    assert_eq!(error.status(), Some(401));
    assert_eq!(subject.transport.requests.len(), 3);
    assert!(subject.output.messages.is_empty());
    assert!(subject.files.writes.is_empty());
}

#[tokio::test]
async fn a_device_transport_failure_is_a_400() {
    let mut subject = authenticator(
        Files::default(),
        vec![Err(TransportFailure::Unavailable)],
        Vec::new(),
    );

    let error = subject.login().await.unwrap_err();

    assert!(matches!(
        &error,
        DeviceAuthError::DeviceCode(DeviceCodeFailure::Transport(_))
    ));
    assert_eq!(error.status(), Some(400));
    assert_eq!(subject.transport.requests.len(), 1);
}

#[tokio::test]
async fn a_device_status_failure_is_a_400() {
    let mut subject = authenticator(
        Files::default(),
        vec![Ok(reply(404, &object([("message", text("not found"))])))],
        Vec::new(),
    );

    let error = subject.login().await.unwrap_err();

    assert!(matches!(
        &error,
        DeviceAuthError::DeviceCode(DeviceCodeFailure::Status { status: 404 })
    ));
    assert_eq!(error.status(), Some(400));
}

#[tokio::test]
async fn a_malformed_device_reply_is_a_400() {
    let mut subject = authenticator(
        Files::default(),
        vec![Ok(raw_reply(200, b"not json"))],
        Vec::new(),
    );

    let error = subject.login().await.unwrap_err();

    assert!(matches!(
        &error,
        DeviceAuthError::DeviceCode(DeviceCodeFailure::Reply(ReplyError::Json(_)))
    ));
    assert_eq!(error.status(), Some(400));
}

#[tokio::test]
async fn an_access_transport_failure_is_a_400_without_a_wait() {
    let mut subject = authenticator(
        Files::default(),
        vec![Ok(device_reply())],
        vec![Err(TransportFailure::Unavailable)],
    );

    let error = subject.login().await.unwrap_err();

    assert!(matches!(
        &error,
        DeviceAuthError::AccessToken(AccessTokenFailure::Transport(_))
    ));
    assert_eq!(error.status(), Some(400));
    assert!(subject.sleeper.sleeps.is_empty());
}

#[tokio::test]
async fn an_access_status_failure_is_a_400_without_a_wait() {
    let mut subject = authenticator(
        Files::default(),
        vec![Ok(device_reply())],
        vec![Ok(reply(500, &object([("message", text("boom"))])))],
    );

    let error = subject.login().await.unwrap_err();

    assert!(matches!(
        &error,
        DeviceAuthError::AccessToken(AccessTokenFailure::Status { status: 500 })
    ));
    assert_eq!(error.status(), Some(400));
    assert!(subject.sleeper.sleeps.is_empty());
}

#[tokio::test]
async fn a_malformed_access_reply_is_a_400() {
    let mut subject = authenticator(
        Files::default(),
        vec![Ok(device_reply())],
        vec![Ok(raw_reply(200, b"[]"))],
    );

    let error = subject.login().await.unwrap_err();

    assert!(matches!(
        &error,
        DeviceAuthError::AccessToken(AccessTokenFailure::Reply(ReplyError::Json(_)))
    ));
    assert_eq!(error.status(), Some(400));
}

#[tokio::test]
async fn an_empty_access_token_succeeds_and_is_written() {
    let mut subject = authenticator(
        Files::default(),
        vec![Ok(device_reply())],
        vec![Ok(token_reply(""))],
    );

    let token = subject.get_access_token().await.unwrap();

    assert_eq!(token.expose_secret(), "");
    assert_eq!(subject.files.writes, vec![Vec::<u8>::new()]);
    assert!(subject.sleeper.sleeps.is_empty());
}

#[tokio::test]
async fn an_unexpected_provider_error_waits_and_retries_the_poll() {
    let mut subject = authenticator(
        Files::default(),
        vec![Ok(device_reply())],
        vec![
            Ok(reply(200, &object([("error", text("slow_down"))]))),
            Ok(token_reply("eventual")),
        ],
    );

    let token = subject.get_access_token().await.unwrap();

    assert_eq!(token.expose_secret(), "eventual");
    assert_eq!(subject.sleeper.sleeps, vec![Duration::from_secs(5)]);
}

#[tokio::test]
async fn a_failed_token_write_returns_the_acquired_token() {
    let files = Files {
        content: None,
        fault: Fault::Write,
        writes: Vec::new(),
        dirs: Vec::new(),
    };
    let mut subject = authenticator(
        files,
        vec![Ok(device_reply())],
        vec![Ok(token_reply("unsaved"))],
    );

    let token = subject.get_access_token().await.unwrap();

    assert_eq!(token.expose_secret(), "unsaved");
    assert_eq!(subject.files.writes, vec![b"unsaved".to_vec()]);
    assert_eq!(subject.transport.requests.len(), 2);
}

#[test]
fn the_constructor_creates_the_directory_and_derives_paths() {
    let subject = authenticator(Files::default(), Vec::new(), Vec::new());

    assert_eq!(subject.files.dirs, vec![PathBuf::from("/tokens")]);
    assert_eq!(
        subject.access_token_path,
        PathBuf::from("/tokens/access-token")
    );
    assert_eq!(subject.api_key_path, PathBuf::from("/tokens/api-key.json"));
}

#[test]
fn the_constructor_reports_a_directory_failure() {
    let files = Files {
        content: None,
        fault: Fault::Directory,
        writes: Vec::new(),
        dirs: Vec::new(),
    };

    let error = Authenticator::new(
        settings(),
        files,
        ScriptSleeper::default(),
        scripted(Vec::new(), Vec::new()),
        ScriptOutput::default(),
    )
    .unwrap_err();

    assert!(matches!(&error, DeviceAuthError::TokenDirectory(_)));
    assert_eq!(error.status(), None);
}

#[test]
fn github_headers_carry_the_oauth_identity() {
    let headers = github_headers(None);

    assert_eq!(headers.len(), 6);
    assert_eq!(headers["accept"], "application/json");
    assert_eq!(headers["editor-version"], "vscode/1.85.1");
    assert_eq!(headers["editor-plugin-version"], "copilot/1.155.0");
    assert_eq!(headers["user-agent"], "GithubCopilot/1.155.0");
    assert_eq!(headers["accept-encoding"], "gzip,deflate,br");
    assert_eq!(headers["content-type"], "application/json");
    assert!(!headers.contains_key("authorization"));
}

#[test]
fn github_headers_use_the_literal_token_scheme() {
    let token = SecretString::from("access-value");

    let headers = github_headers(Some(&token));

    assert_eq!(headers.len(), 7);
    assert_eq!(headers["authorization"], "token access-value");
}

#[test]
fn copilot_headers_match_the_inference_identity() {
    let request_id = Uuid::parse_str("6f8e2e60-4b90-4f0f-9f6f-1c1a3f2d4e5a").unwrap();

    let headers = copilot_default_headers(&SecretString::from("api-key"), Some(request_id));

    assert_eq!(headers.len(), 10);
    assert_eq!(headers["Authorization"], "Bearer api-key");
    assert_eq!(headers["content-type"], "application/json");
    assert_eq!(headers["copilot-integration-id"], "vscode-chat");
    assert_eq!(headers["editor-version"], "vscode/1.95.0");
    assert_eq!(headers["editor-plugin-version"], "copilot-chat/0.26.7");
    assert_eq!(headers["user-agent"], "GitHubCopilotChat/0.26.7");
    assert_eq!(headers["openai-intent"], "conversation-panel");
    assert_eq!(headers["x-github-api-version"], "2025-04-01");
    assert_eq!(
        headers["x-request-id"],
        "6f8e2e60-4b90-4f0f-9f6f-1c1a3f2d4e5a"
    );
    assert_eq!(
        headers["x-vscode-user-agent-library-version"],
        "electron-fetch"
    );
}

#[test]
fn copilot_headers_generate_a_fresh_request_id() {
    let key = SecretString::from("api-key");

    let first = copilot_default_headers(&key, None);
    let second = copilot_default_headers(&key, None);

    let first_id = Uuid::parse_str(&first["x-request-id"]).unwrap();
    let second_id = Uuid::parse_str(&second["x-request-id"]).unwrap();
    assert_ne!(first_id, second_id);
    assert_eq!(first_id.get_version_num(), 4);
}

#[test]
fn copilot_constants_match_the_source() {
    assert_eq!(COPILOT_VERSION, "0.26.7");
    assert_eq!(EDITOR_PLUGIN_VERSION, "copilot-chat/0.26.7");
    assert_eq!(USER_AGENT, "GitHubCopilotChat/0.26.7");
    assert_eq!(API_VERSION, "2025-04-01");
    assert_eq!(DEFAULT_COPILOT_API_BASE, "https://api.githubcopilot.com");
    assert!(EDITOR_PLUGIN_VERSION.ends_with(COPILOT_VERSION));
    assert!(USER_AGENT.ends_with(COPILOT_VERSION));
}

#[test]
fn device_reply_decodes_typed_and_additional_fields() {
    let reply: DeviceReply = super::decode::decode_device_reply(
        br#"{"device_code":"d","user_code":"u","verification_uri":"v","interval":5,"expires_in":900}"#,
    )
    .unwrap();

    assert_eq!(reply.device_code.as_deref(), Some("d"));
    assert_eq!(reply.user_code.as_deref(), Some("u"));
    assert_eq!(reply.verification_uri.as_deref(), Some("v"));
    assert_eq!(reply.additional_fields.get("interval"), Some(&json!(5_i32)));
    assert_eq!(
        reply.additional_fields.get("expires_in"),
        Some(&json!(900_i32))
    );
    assert_eq!(reply.additional_fields.len(), 2);
}

#[rstest]
#[case::array(b"[]".as_slice())]
#[case::truncated(b"{".as_slice())]
#[case::wrong_type(br#"{"device_code": 5}"#.as_slice())]
fn device_reply_rejects_malformed_json(#[case] bytes: &[u8]) {
    assert!(matches!(
        super::decode::decode_device_reply(bytes),
        Err(ReplyError::Json(_))
    ));
}

#[test]
fn device_reply_is_bounded_by_its_limit() {
    let bounded = format!("{{}}{}", " ".repeat(REPLY_LIMIT - 2));
    super::decode::decode_device_reply(bounded.as_bytes()).unwrap();
    assert!(matches!(
        super::decode::decode_device_reply(format!("{bounded} ").as_bytes()),
        Err(ReplyError::Size { limit: REPLY_LIMIT })
    ));
}

#[test]
fn access_reply_reads_presence_and_accepts_an_empty_token() {
    let empty: AccessReply = super::decode::decode_access_reply(br#"{"access_token":""}"#).unwrap();
    assert_eq!(empty.access_token.unwrap().expose_secret(), "");

    let missing: AccessReply = super::decode::decode_access_reply(b"{}").unwrap();
    assert!(missing.access_token.is_none());
    assert!(missing.error.is_none());

    let pending: AccessReply =
        super::decode::decode_access_reply(br#"{"error":"authorization_pending"}"#).unwrap();
    assert_eq!(pending.error.as_deref(), Some("authorization_pending"));
}

#[test]
fn access_reply_is_bounded_by_its_limit() {
    let bounded = format!("{{}}{}", " ".repeat(REPLY_LIMIT - 2));
    super::decode::decode_access_reply(bounded.as_bytes()).unwrap();
    assert!(matches!(
        super::decode::decode_access_reply(format!("{bounded} ").as_bytes()),
        Err(ReplyError::Size { limit: REPLY_LIMIT })
    ));
}

#[rstest]
fn replays_the_device_reply_corpus(#[files("fuzz/corpus/device_reply/*")] path: PathBuf) {
    let bytes = fs::read(path).expect("corpus file");
    let _device = super::decode::decode_device_reply(&bytes);
    let _access = super::decode::decode_access_reply(&bytes);
}

#[test]
fn local_files_write_then_read_and_truncate() {
    let directory = tempfile::tempdir().unwrap();
    let mut files = LocalFiles;
    files.ensure_dir(directory.path()).unwrap();
    let path = directory.path().join("access-token");

    files.write(&path, b"first-token").unwrap();
    assert_eq!(files.read(&path).unwrap(), b"first-token");
    files.write(&path, b"new").unwrap();
    assert_eq!(files.read(&path).unwrap(), b"new");

    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn local_files_create_a_missing_directory_and_skip_an_existing_one() {
    let directory = tempfile::tempdir().unwrap();
    let nested = directory.path().join("tokens");
    let mut files = LocalFiles;

    files.ensure_dir(&nested).unwrap();
    assert!(nested.is_dir());
    files.ensure_dir(&nested).unwrap();
    assert!(files.ensure_dir(Path::new("")).is_err());
}

#[test]
fn local_files_reject_oversized_or_missing_token_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("access-token");
    let mut files = LocalFiles;

    fs::write(&path, vec![b'x'; FILE_LIMIT]).unwrap();
    assert_eq!(files.read(&path).unwrap().len(), FILE_LIMIT);
    fs::write(&path, vec![b'x'; FILE_LIMIT + 1]).unwrap();
    files.read(&path).unwrap_err();
    files.read(&directory.path().join("missing")).unwrap_err();
}

#[test]
fn local_files_report_a_write_into_a_missing_directory() {
    let directory = tempfile::tempdir().unwrap();
    let mut files = LocalFiles;

    assert!(
        files
            .write(&directory.path().join("missing/access-token"), b"x")
            .is_err()
    );
}

#[test]
fn stream_output_writes_the_instruction_line() {
    let mut output = StreamOutput { writer: Vec::new() };

    output.instructions("Please visit https://github.com/login/device and enter code ABCD.");

    assert_eq!(
        output.writer,
        b"Please visit https://github.com/login/device and enter code ABCD.\n"
    );
}

struct FailingWriter;

impl io::Write for FailingWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("closed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(io::Error::other("closed"))
    }
}

#[test]
fn stream_output_logs_a_write_failure() {
    let ((), output) = logged(|| {
        let mut output = StreamOutput {
            writer: FailingWriter,
        };
        output.instructions("Please visit the device page.");
    });

    assert!(output.contains("WARN the Copilot device login instructions could not be written"));
}

#[tokio::test]
async fn tokio_sleeper_waits_for_the_requested_duration() {
    let mut sleeper = TokioSleeper;
    let started = Instant::now();

    sleeper.sleep(Duration::from_millis(20)).await;

    assert!(started.elapsed() >= Duration::from_millis(10));
}

#[test]
fn login_diagnostics_never_contain_the_access_token() {
    let (token, output) = logged(|| {
        block_on(async {
            let mut subject = authenticator(
                Files::default(),
                vec![Ok(device_reply())],
                vec![Ok(token_reply("private-access-token"))],
            );
            subject.get_access_token().await
        })
    });

    assert_eq!(token.unwrap().expose_secret(), "private-access-token");
    assert!(output.contains("the GitHub Copilot device login succeeded"));
    assert!(!output.contains("private-access-token"));
}

#[test]
fn a_write_failure_logs_without_the_acquired_token() {
    let (token, output) = logged(|| {
        block_on(async {
            let mut subject = authenticator(
                Files {
                    content: None,
                    fault: Fault::Write,
                    writes: Vec::new(),
                    dirs: Vec::new(),
                },
                vec![Ok(device_reply())],
                vec![Ok(token_reply("private-access-token"))],
            );
            subject.get_access_token().await
        })
    });

    token.unwrap();
    assert!(output.contains("ERROR the Copilot access token could not be saved"));
    assert!(!output.contains("private-access-token"));
}

#[test]
fn authorization_pending_logs_at_debug() {
    let (result, output) = logged(|| {
        block_on(async {
            let mut subject = authenticator(
                Files::default(),
                vec![Ok(device_reply())],
                vec![Ok(pending_reply()), Ok(token_reply("later"))],
            );
            subject.get_access_token().await
        })
    });

    result.unwrap();
    assert!(output.contains("DEBUG GitHub authorization is pending"));
}

#[test]
fn an_unexpected_reply_logs_a_warning() {
    let (result, output) = logged(|| {
        block_on(async {
            let mut subject = authenticator(
                Files::default(),
                vec![Ok(device_reply())],
                vec![
                    Ok(reply(200, &object([("error", text("slow_down"))]))),
                    Ok(token_reply("later")),
                ],
            );
            subject.get_access_token().await
        })
    });

    result.unwrap();
    assert!(output.contains("WARN GitHub returned an unexpected access token reply"));
}

fn policy() -> Arc<ProxyPolicy> {
    Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap())
}

fn http_transport() -> HttpTransport {
    HttpTransport {
        client: ProxiedClient::outbound(&policy()).unwrap(),
        deadline: Duration::from_secs(5),
    }
}

async fn serve(listener: TcpListener, status: u16, body: &'static str) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n") {
        received.push(stream.read_u8().await.unwrap());
    }
    let head = String::from_utf8(received).unwrap();
    let length: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .map_or(0, |value| value.parse().unwrap());
    let mut sent = vec![0_u8; length];
    stream.read_exact(&mut sent).await.unwrap();
    let reply = format!(
        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _written = stream.write_all(reply.as_bytes()).await;
    format!("{head}{}", String::from_utf8(sent).unwrap())
}

#[tokio::test]
async fn http_transport_posts_json_with_its_headers() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut transport = http_transport();
    let url = format!("http://{address}/login/device/code");
    let body = br#"{"client_id":"c","scope":"read:user"}"#.to_vec();
    let post = Post {
        url: &url,
        headers: github_headers(None),
        body,
    };

    let (received, answered) = tokio::join!(
        serve(listener, 200, r#"{"device_code":"d"}"#),
        transport.post_json(post)
    );

    let reply = answered.unwrap();
    assert_eq!(reply.status, 200);
    assert_eq!(reply.body, br#"{"device_code":"d"}"#);
    assert!(received.starts_with("POST /login/device/code HTTP/1.1\r\n"));
    assert!(received.contains("accept: application/json"));
    assert!(received.contains("editor-version: vscode/1.85.1"));
    assert!(received.contains("content-type: application/json"));
    assert!(received.ends_with(r#"{"client_id":"c","scope":"read:user"}"#));
}

#[tokio::test]
async fn http_transport_returns_error_statuses_as_replies() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut transport = http_transport();
    let url = format!("http://{address}/login/oauth/access_token");
    let post = Post {
        url: &url,
        headers: Headers::new(),
        body: b"{}".to_vec(),
    };

    let (_received, answered) = tokio::join!(
        serve(listener, 400, r#"{"error":"bad_verification_code"}"#),
        transport.post_json(post)
    );

    let reply = answered.unwrap();
    assert_eq!(reply.status, 400);
}

#[tokio::test]
async fn http_transport_rejects_an_invalid_url() {
    let mut transport = http_transport();
    let post = Post {
        url: "not a url",
        headers: Headers::new(),
        body: Vec::new(),
    };

    let error = transport.post_json(post).await.unwrap_err();

    assert!(matches!(error, TransportFailure::Url(_)));
}

#[tokio::test]
async fn http_transport_rejects_an_invalid_header_name() {
    let mut transport = http_transport();
    let mut headers = Headers::new();
    headers.insert(String::from("bad header"), String::from("value"));
    let post = Post {
        url: "http://127.0.0.1:1/login/device/code",
        headers,
        body: Vec::new(),
    };

    let error = transport.post_json(post).await.unwrap_err();

    assert!(matches!(
        error,
        TransportFailure::HeaderName { name, .. } if name == "bad header"
    ));
}

#[tokio::test]
async fn http_transport_rejects_an_invalid_header_value() {
    let mut transport = http_transport();
    let mut headers = Headers::new();
    headers.insert(String::from("accept"), String::from("bad\nvalue"));
    let post = Post {
        url: "http://127.0.0.1:1/login/device/code",
        headers,
        body: Vec::new(),
    };

    let error = transport.post_json(post).await.unwrap_err();

    assert!(matches!(
        error,
        TransportFailure::HeaderValue { name, .. } if name == "accept"
    ));
}

#[tokio::test]
async fn http_transport_reports_a_refused_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let mut transport = http_transport();
    let url = format!("http://{address}/login/device/code");
    let post = Post {
        url: &url,
        headers: Headers::new(),
        body: Vec::new(),
    };

    let error = transport.post_json(post).await.unwrap_err();

    assert!(matches!(error, TransportFailure::Request(_)));
}

#[test]
fn post_debug_hides_header_values() {
    let mut headers = Headers::new();
    headers.insert(
        String::from("authorization"),
        String::from("token private-access-token"),
    );
    let post = Post {
        url: DEVICE_URL,
        headers,
        body: b"{}".to_vec(),
    };

    let debug = format!("{post:?}");

    assert!(!debug.contains("private-access-token"));
    assert!(debug.contains("authorization"));
    assert!(debug.contains("body_bytes"));
}
