#![cfg(test)]

use std::collections::VecDeque;
use std::fs;
use std::future::{Future, ready};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use betterlite_config::sections::copilot_device::Settings;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use tokio::runtime::Builder as RuntimeBuilder;
use tracing::subscriber::with_default;

use super::clock::Clock;
use super::error::ApiKeyError;
use super::transport::{ApiKeyTransport, Get};
use super::{Authenticator, REFRESH_ATTEMPTS};
use crate::copilot::device_auth::error::DeviceAuthError;
use crate::copilot::device_auth::files::FileAccess;
use crate::copilot::device_auth::github_headers;
use crate::copilot::device_auth::output::OperatorOutput;
use crate::copilot::device_auth::sleeper::Sleeper;
use crate::copilot::device_auth::transport::{Post, Reply, Transport, TransportFailure};
use crate::openai_base_config::Headers;

const NOW: i64 = 1_700_000_000;
const KEY_URL: &str = "https://key.example/token";
const DEVICE_URL: &str = "https://device.example/code";
const ACCESS_URL: &str = "https://token.example/access";
const API_KEY_PATH: &str = "/tokens/api-key.json";

fn settings() -> Settings {
    Settings {
        token_dir: String::from("/tokens"),
        access_token_file: String::from("access-token"),
        api_key_file: String::from("api-key.json"),
        client_id: String::from("Iv1.test"),
        device_code_url: String::from(DEVICE_URL),
        access_token_url: String::from(ACCESS_URL),
        api_key_url: String::from(KEY_URL),
    }
}

#[derive(Debug, Default)]
struct Files {
    api_key: Option<Vec<u8>>,
    access_token: Option<Vec<u8>>,
    fail_write: bool,
    reads: Vec<PathBuf>,
    writes: Vec<(PathBuf, Vec<u8>)>,
}

impl Files {
    fn given(api_key: Option<&str>, access_token: Option<&str>, fail_write: bool) -> Self {
        Self {
            api_key: api_key.map(|text| text.as_bytes().to_vec()),
            access_token: access_token.map(|text| text.as_bytes().to_vec()),
            fail_write,
            reads: Vec::new(),
            writes: Vec::new(),
        }
    }
}

impl FileAccess for Files {
    fn ensure_dir(&mut self, _path: &Path) -> io::Result<()> {
        Ok(())
    }

    fn read(&mut self, path: &Path) -> impl Future<Output = io::Result<Vec<u8>>> + Send {
        self.reads.push(path.to_path_buf());
        let content = if path == Path::new(API_KEY_PATH) {
            &self.api_key
        } else {
            &self.access_token
        };
        ready(
            content
                .clone()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound)),
        )
    }

    fn write(
        &mut self,
        path: &Path,
        content: &[u8],
    ) -> impl Future<Output = io::Result<()>> + Send {
        if self.fail_write {
            return ready(Err(io::Error::other("scripted write failure")));
        }
        self.writes.push((path.to_path_buf(), content.to_vec()));
        ready(Ok(()))
    }
}

#[derive(Debug)]
struct ScriptClock {
    sleeps: Vec<Duration>,
}

impl Clock for ScriptClock {
    fn now(&self) -> i64 {
        NOW
    }
}

impl Sleeper for ScriptClock {
    fn sleep(&mut self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.sleeps.push(duration);
        ready(())
    }
}

#[derive(Debug, Default)]
struct Scripted {
    replies: VecDeque<Result<Reply, TransportFailure>>,
    gets: Vec<(String, Headers)>,
    posts: Vec<String>,
}

impl ApiKeyTransport for Scripted {
    fn get<'call>(
        &'call mut self,
        get: Get<'call>,
    ) -> impl Future<Output = Result<Reply, TransportFailure>> + Send + 'call {
        self.gets.push((get.url.to_owned(), get.headers));
        ready(
            self.replies
                .pop_front()
                .unwrap_or(Err(TransportFailure::Unavailable)),
        )
    }
}

impl Transport for Scripted {
    fn post_json<'call>(
        &'call mut self,
        post: Post<'call>,
    ) -> impl Future<Output = Result<Reply, TransportFailure>> + Send + 'call {
        self.posts.push(post.url.to_owned());
        ready(Err(TransportFailure::Unavailable))
    }
}

#[derive(Debug, Default)]
struct ScriptOutput;

impl OperatorOutput for ScriptOutput {
    fn instructions(&mut self, _message: &str) {}
}

type Subject = Authenticator<Files, ScriptClock, Scripted, ScriptOutput>;

fn authenticator(files: Files, replies: Vec<Result<Reply, TransportFailure>>) -> Subject {
    Authenticator::new(
        settings(),
        files,
        ScriptClock { sleeps: Vec::new() },
        Scripted {
            replies: VecDeque::from(replies),
            gets: Vec::new(),
            posts: Vec::new(),
        },
        ScriptOutput,
    )
    .unwrap()
}

fn reply(status: u16, body: &str) -> Reply {
    Reply {
        status,
        body: body.as_bytes().to_vec(),
    }
}

fn key_reply(token: &str, expires_at: i64) -> Reply {
    reply(
        200,
        &format!(
            r#"{{"token":"{token}","expires_at":{expires_at},"endpoints":{{"api":"https://api.example"}}}}"#
        ),
    )
}

fn value(source: &str) -> Value {
    serde_json::from_str(source).unwrap()
}

fn logged<T, F>(action: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
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
async fn a_fresh_cached_key_is_returned_without_a_request() {
    let mut subject = authenticator(
        Files::given(
            Some(r#"{"token":"cached","expires_at":1700000001}"#),
            None,
            false,
        ),
        Vec::new(),
    );

    let token = subject.get_api_key().await.unwrap();

    assert_eq!(token.unwrap().expose_secret(), "cached");
    assert!(subject.transport.gets.is_empty());
    assert!(subject.transport.posts.is_empty());
    assert!(subject.files.writes.is_empty());
    assert_eq!(subject.files.reads, vec![PathBuf::from(API_KEY_PATH)]);
}

#[tokio::test]
async fn a_fresh_record_without_a_token_returns_none() {
    let mut subject = authenticator(
        Files::given(Some(r#"{"expires_at":1700000001}"#), None, false),
        Vec::new(),
    );

    let token = subject.get_api_key().await.unwrap();

    assert!(token.is_none());
    assert!(subject.transport.gets.is_empty());
    assert!(subject.files.writes.is_empty());
}

#[tokio::test]
async fn an_expiry_equal_to_now_refreshes() {
    let mut subject = authenticator(
        Files::given(
            Some(r#"{"token":"cached","expires_at":1700000000}"#),
            Some("access-token"),
            false,
        ),
        vec![Ok(key_reply("fresh", 1_800_000_000))],
    );

    let token = subject.get_api_key().await.unwrap();

    assert_eq!(token.unwrap().expose_secret(), "fresh");
    assert_eq!(subject.transport.gets.len(), 1);
    assert_eq!(subject.files.writes.len(), 1);
}

#[tokio::test]
async fn an_expired_record_is_replaced_by_the_refreshed_one() {
    let mut subject = authenticator(
        Files::given(
            Some(
                r#"{"token":"cached","expires_at":1699999999,"endpoints":{"api":"https://old.example","proxy":"https://proxy.example"},"sku":"copilot-pro"}"#,
            ),
            Some("access-token"),
            false,
        ),
        vec![Ok(key_reply("fresh", 1_800_000_000))],
    );

    let token = subject.get_api_key().await.unwrap();

    assert_eq!(token.unwrap().expose_secret(), "fresh");
    let (path, bytes) = &subject.files.writes[0];
    assert_eq!(path, &PathBuf::from(API_KEY_PATH));
    let written: Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(
        written,
        value(
            r#"{"token":"fresh","expires_at":1800000000,"endpoints":{"api":"https://api.example"}}"#
        )
    );
}

#[test]
fn a_missing_api_key_file_refreshes() {
    let (token, output) = logged(|| {
        block_on(async {
            let mut subject = authenticator(
                Files::given(None, Some("access-token"), false),
                vec![Ok(key_reply("fresh", 1_800_000_000))],
            );
            subject.get_api_key().await
        })
    });

    assert_eq!(token.unwrap().unwrap().expose_secret(), "fresh");
    assert!(output.contains("WARN no Copilot API key file was found"));
}

#[test]
fn a_malformed_api_key_file_warns_and_refreshes() {
    let (token, output) = logged(|| {
        block_on(async {
            let mut subject = authenticator(
                Files::given(Some("{not json"), Some("access-token"), false),
                vec![Ok(key_reply("fresh", 1_800_000_000))],
            );
            subject.get_api_key().await
        })
    });

    assert_eq!(token.unwrap().unwrap().expose_secret(), "fresh");
    assert!(output.contains("WARN the Copilot API key file could not be decoded"));
}

#[tokio::test]
async fn a_wrong_typed_token_never_becomes_a_key() {
    let mut subject = authenticator(
        Files::given(
            Some(r#"{"token":5,"expires_at":1700000001}"#),
            Some("access-token"),
            false,
        ),
        vec![Ok(key_reply("fresh", 1_800_000_000))],
    );

    let token = subject.get_api_key().await.unwrap();

    assert_eq!(token.unwrap().expose_secret(), "fresh");
    assert_eq!(subject.transport.gets.len(), 1);
}

#[tokio::test]
async fn the_third_attempt_returns_the_key() {
    let mut subject = authenticator(
        Files::given(None, Some("access-token"), false),
        vec![
            Ok(reply(500, r#"{"message":"busy"}"#)),
            Ok(reply(200, r#"{"expires_at":1800000000}"#)),
            Ok(key_reply("third", 1_800_000_000)),
        ],
    );

    let token = subject.get_api_key().await.unwrap();

    assert_eq!(token.unwrap().expose_secret(), "third");
    assert_eq!(subject.transport.gets.len(), usize::from(REFRESH_ATTEMPTS));
    assert_eq!(subject.files.writes.len(), 1);
}

#[tokio::test]
async fn three_tokenless_replies_are_a_401() {
    let mut subject = authenticator(
        Files::given(None, Some("access-token"), false),
        vec![
            Ok(reply(500, r#"{"message":"busy"}"#)),
            Ok(reply(200, r#"{"expires_at":1800000000}"#)),
            Ok(reply(200, r#"{"expires_at":1800000000}"#)),
        ],
    );

    let error = subject.get_api_key().await.unwrap_err();

    assert!(matches!(error, ApiKeyError::Refresh));
    assert_eq!(error.status(), 401);
    assert_eq!(subject.transport.gets.len(), usize::from(REFRESH_ATTEMPTS));
    assert!(subject.files.writes.is_empty());
}

#[tokio::test]
async fn a_malformed_key_reply_consumes_an_attempt() {
    let mut subject = authenticator(
        Files::given(None, Some("access-token"), false),
        vec![
            Ok(reply(
                200,
                r#"{"token":"first","expires_at":1800000000,"endpoints":[]}"#,
            )),
            Ok(key_reply("second", 1_800_000_000)),
        ],
    );

    let token = subject.get_api_key().await.unwrap();

    assert_eq!(token.unwrap().expose_secret(), "second");
    assert_eq!(subject.transport.gets.len(), 2);
}

#[tokio::test]
async fn a_transport_failure_consumes_an_attempt() {
    let mut subject = authenticator(
        Files::given(None, Some("access-token"), false),
        vec![
            Err(TransportFailure::Unavailable),
            Ok(key_reply("eventual", 1_800_000_000)),
        ],
    );

    let token = subject.get_api_key().await.unwrap();

    assert_eq!(token.unwrap().expose_secret(), "eventual");
    assert_eq!(subject.transport.gets.len(), 2);
}

#[tokio::test]
async fn an_empty_token_is_written_before_the_401() {
    let mut subject = authenticator(
        Files::given(None, Some("access-token"), false),
        vec![Ok(reply(200, r#"{"token":""}"#))],
    );

    let error = subject.get_api_key().await.unwrap_err();

    assert!(matches!(error, ApiKeyError::MissingToken));
    assert_eq!(error.status(), 401);
    let written: Value = serde_json::from_slice(&subject.files.writes[0].1).unwrap();
    assert_eq!(written, value(r#"{"token":""}"#));
}

#[tokio::test]
async fn a_failed_write_is_a_500_and_leaves_the_file_alone() {
    let mut subject = authenticator(
        Files::given(None, Some("access-token"), true),
        vec![Ok(key_reply("unsaved", 1_800_000_000))],
    );

    let error = subject.get_api_key().await.unwrap_err();

    assert!(matches!(error, ApiKeyError::Save(_)));
    assert_eq!(error.status(), 500);
    assert!(subject.files.writes.is_empty());
}

#[tokio::test]
async fn an_unavailable_access_token_is_a_401() {
    let mut subject = authenticator(Files::given(None, None, false), Vec::new());

    let error = subject.get_api_key().await.unwrap_err();

    assert!(matches!(
        error,
        ApiKeyError::AccessToken(DeviceAuthError::Exhausted)
    ));
    assert_eq!(error.status(), 401);
    assert!(subject.transport.gets.is_empty());
    assert_eq!(subject.transport.posts.len(), 3);
}

#[tokio::test]
async fn the_refresh_sends_the_access_token_with_the_literal_scheme() {
    let mut subject = authenticator(
        Files::given(None, Some("access-token"), false),
        vec![Ok(key_reply("fresh", 1_800_000_000))],
    );

    subject.get_api_key().await.unwrap();

    let (url, headers) = &subject.transport.gets[0];
    assert_eq!(url, KEY_URL);
    assert_eq!(
        headers,
        &github_headers(Some(&SecretString::from("access-token")))
    );
}

#[tokio::test]
async fn the_access_token_is_read_once_for_three_attempts() {
    let mut subject = authenticator(
        Files::given(None, Some("access-token"), false),
        vec![
            Ok(reply(200, r#"{"expires_at":1800000000}"#)),
            Ok(reply(200, r#"{"expires_at":1800000000}"#)),
            Ok(reply(200, r#"{"expires_at":1800000000}"#)),
        ],
    );

    subject.get_api_key().await.unwrap_err();

    assert_eq!(
        subject.files.reads,
        vec![
            PathBuf::from(API_KEY_PATH),
            PathBuf::from("/tokens/access-token")
        ]
    );
}

#[tokio::test]
async fn get_api_base_returns_the_endpoint_of_an_expired_record() {
    let mut subject = authenticator(
        Files::given(
            Some(
                r#"{"token":"cached","expires_at":1,"endpoints":{"api":"https://api.example","proxy":"https://proxy.example"}}"#,
            ),
            None,
            false,
        ),
        Vec::new(),
    );

    let base = subject.get_api_base().await;

    assert_eq!(base.as_deref(), Some("https://api.example"));
    assert!(subject.transport.gets.is_empty());
    assert!(subject.transport.posts.is_empty());
    assert!(subject.files.writes.is_empty());
    assert_eq!(subject.files.reads, vec![PathBuf::from(API_KEY_PATH)]);
}

#[test]
fn get_api_base_returns_none_without_a_file() {
    let (base, output) = logged(|| {
        block_on(async {
            let mut subject = authenticator(Files::given(None, None, false), Vec::new());
            subject.get_api_base().await
        })
    });

    assert!(base.is_none());
    assert!(output.contains("WARN no Copilot API key file was found"));
}

#[test]
fn get_api_base_returns_none_for_a_malformed_file() {
    let (base, output) = logged(|| {
        block_on(async {
            let mut subject =
                authenticator(Files::given(Some("{not json"), None, false), Vec::new());
            subject.get_api_base().await
        })
    });

    assert!(base.is_none());
    assert!(output.contains("WARN the Copilot API key file could not be decoded"));
}

#[tokio::test]
async fn get_api_base_returns_none_without_an_endpoint() {
    let mut subject = authenticator(
        Files::given(Some(r#"{"token":"cached","endpoints":{}}"#), None, false),
        Vec::new(),
    );

    assert!(subject.get_api_base().await.is_none());
}

#[test]
fn diagnostics_never_contain_the_key_or_the_access_token() {
    let (token, output) = logged(|| {
        block_on(async {
            let mut subject = authenticator(
                Files::given(
                    Some(r#"{"token":"private-cached-key","expires_at":1}"#),
                    Some("private-access-token"),
                    false,
                ),
                vec![
                    Ok(reply(500, r#"{"message":"busy"}"#)),
                    Ok(key_reply("private-fresh-key", 1_800_000_000)),
                ],
            );
            subject.get_api_key().await
        })
    });

    assert_eq!(token.unwrap().unwrap().expose_secret(), "private-fresh-key");
    assert!(output.contains("the Copilot API key expired"));
    assert!(output.contains("the Copilot API key request failed"));
    assert!(!output.contains("private-cached-key"));
    assert!(!output.contains("private-fresh-key"));
    assert!(!output.contains("private-access-token"));
}

#[test]
fn api_key_statuses_match_the_source_family() {
    assert_eq!(ApiKeyError::Refresh.status(), 401);
    assert_eq!(ApiKeyError::MissingToken.status(), 401);
    assert_eq!(
        ApiKeyError::AccessToken(DeviceAuthError::Exhausted).status(),
        401
    );
    assert_eq!(ApiKeyError::Save(io::Error::other("denied")).status(), 500);
    assert_eq!(
        ApiKeyError::Encode(serde_json::from_str::<Value>("").unwrap_err()).status(),
        500
    );
}
