#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::fs;
use std::future::pending;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::anthropic_beta_config::Settings;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use tokio::time::Instant;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
use tracing_subscriber::registry;

use super::BUNDLED_CONFIG;
use super::BetaConfigLoader;
use super::error::{BetaConfigError, BetaShapeError};
use super::matrix::{BetaMatrix, BetaNode, decode_beta_matrix, validate_beta_matrix};
use crate::api::api_boundaries::{ApiLimitError, ApiLimits};

const PROVIDERS: [&str; 7] = [
    "anthropic",
    "azure_ai",
    "bedrock_converse",
    "bedrock",
    "bedrock_mantle",
    "vertex_ai",
    "databricks",
];
const EMPTY_SIX: &str = r#"{"anthropic":{},"azure_ai":{},"bedrock":{},"bedrock_converse":{},"vertex_ai":{},"provider_aliases":{}}"#;

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<(Level, String)>>>);

impl Capture {
    fn events(&self) -> Vec<(Level, String)> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
struct KindVisitor(String);

impl Visit for KindVisitor {
    fn record_debug(&mut self, _field: &Field, _value: &dyn Debug) {}

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "kind" {
            value.clone_into(&mut self.0);
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Capture {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut visitor = KindVisitor::default();
        event.record(&mut visitor);
        if !visitor.0.is_empty() {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((*event.metadata().level(), visitor.0));
        }
    }
}

fn capture() -> (Capture, DefaultGuard) {
    let logs = Capture::default();
    let guard = set_default(registry().with(logs.clone()));
    (logs, guard)
}

enum Reply {
    Answer(u16, &'static str),
    Stall,
}

// Each request records its first line and how many diagnostics existed when it arrived.
struct Mock {
    url: String,
    requests: Arc<Mutex<Vec<(String, usize)>>>,
    _tasks: JoinSet<()>,
}

impl Mock {
    fn request_count(&self) -> usize {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }
}

async fn answer(mut socket: TcpStream, status: u16, body: &str) {
    let head = format!(
        "HTTP/1.1 {status} Status\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    socket.write_all(head.as_bytes()).await.unwrap();
    drop(socket.shutdown().await);
}

async fn serve(replies: Vec<Reply>, logs: Capture) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/matrix.json", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0_u8; 4096];
            let read = socket.read(&mut buffer).await.unwrap();
            buffer.truncate(read);
            let first_line = String::from_utf8(buffer)
                .unwrap()
                .lines()
                .next()
                .unwrap()
                .to_owned();
            let count = {
                let mut log = seen.lock().unwrap_or_else(PoisonError::into_inner);
                log.push((first_line, logs.events().len()));
                log.len()
            };
            match replies
                .get(count.saturating_sub(1))
                .or_else(|| replies.last())
            {
                Some(Reply::Answer(status, body)) => answer(socket, *status, body).await,
                Some(Reply::Stall) | None => pending::<()>().await,
            }
        }
    });
    Mock {
        url,
        requests,
        _tasks: tasks,
    }
}

fn loader(local_only: bool, url: &str) -> BetaConfigLoader {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let settings = Settings {
        local_only,
        url: url.to_owned(),
    };
    BetaConfigLoader::new(
        ProxiedClient::outbound(&policy).unwrap(),
        settings,
        ApiLimits::default(),
    )
}

fn matrix(text: &str) -> BetaMatrix {
    serde_json::from_str(text).unwrap()
}

fn header<'matrix>(
    matrix: &'matrix BetaMatrix,
    provider: &str,
    name: &str,
) -> Option<&'matrix BetaNode> {
    match matrix.0.get(provider) {
        Some(BetaNode::Map(headers)) => headers.get(name),
        _ => None,
    }
}

// The cache fill is always the last diagnostic and every warning before it is a WARN level event.
fn warnings(logs: &Capture) -> Vec<String> {
    let events = logs.events();
    let (last, before) = events.split_last().unwrap();
    assert_eq!(last, &(Level::DEBUG, "cache_filled".to_owned()));
    assert!(before.iter().all(|(level, _)| *level == Level::WARN));
    before.iter().map(|(_, kind)| kind.clone()).collect()
}

#[tokio::test]
async fn local_matrix_snapshot() {
    let (logs, _guard) = capture();
    let mock = serve(vec![Reply::Answer(200, "{}")], logs.clone()).await;
    let loaded = loader(true, &mock.url).matrix().await;
    assert_eq!(*loaded, matrix(BUNDLED_CONFIG));
    assert_eq!(BUNDLED_CONFIG.lines().count(), 253);
    let keys: Vec<&str> = loaded.0.keys().map(String::as_str).collect();
    assert_eq!(keys.first(), Some(&"description"));
    assert_eq!(keys.get(1..), Some(PROVIDERS.as_slice()));
    let text = |name: &str| BetaNode::Text(name.to_owned());
    assert_eq!(
        header(&loaded, "bedrock", "advanced-tool-use-2025-11-20"),
        Some(&text("tool-search-tool-2025-10-19"))
    );
    assert_eq!(
        header(&loaded, "anthropic", "compact-2026-09-04"),
        Some(&text("compact-2026-09-04"))
    );
    assert_eq!(
        header(&loaded, "azure_ai", "compact-2026-01-12"),
        Some(&BetaNode::Null)
    );
    assert_eq!(
        header(&loaded, "bedrock_mantle", "claude-code-20250219"),
        Some(&text("claude-code-20250219"))
    );
    assert_eq!(mock.request_count(), 0);
    assert_eq!(warnings(&logs), Vec::<String>::new());
}

#[rstest]
#[case::empty_provider(r#"{"anthropic":{}}"#, &[])]
#[case::null_provider(r#"{"anthropic":null}"#, &[])]
#[case::azure_ai(r#"{"azure_ai":{}}"#, &[])]
#[case::bedrock(r#"{"bedrock":{}}"#, &[])]
#[case::bedrock_converse(r#"{"bedrock_converse":{}}"#, &[])]
#[case::vertex_ai(r#"{"vertex_ai":{}}"#, &[])]
#[case::array("[]", &["not_a_dict", "integrity_failed"])]
#[case::text(r#""anthropic""#, &["not_a_dict", "integrity_failed"])]
#[case::null("null", &["not_a_dict", "integrity_failed"])]
#[case::empty_object("{}", &["empty", "integrity_failed"])]
#[case::aliases_only(r#"{"provider_aliases":{}}"#, &["missing_provider_keys", "integrity_failed"])]
#[case::databricks_only(r#"{"databricks":{}}"#, &["missing_provider_keys", "integrity_failed"])]
#[case::mantle_only(r#"{"bedrock_mantle":{}}"#, &["missing_provider_keys", "integrity_failed"])]
#[tokio::test]
async fn matrix_validation(#[case] body: &'static str, #[case] expected: &[&str]) {
    let (logs, _guard) = capture();
    let mock = serve(vec![Reply::Answer(200, body)], logs.clone()).await;
    let loaded = loader(false, &mock.url).matrix().await;
    let wanted = if expected.is_empty() {
        matrix(body)
    } else {
        matrix(BUNDLED_CONFIG)
    };
    assert_eq!(*loaded, wanted);
    assert_eq!(warnings(&logs), expected);
}

#[rstest]
#[case::array("[]", "array")]
#[case::null("null", "null")]
#[case::flag("true", "boolean")]
#[case::number("5", "number")]
#[case::text(r#""anthropic""#, "string")]
fn shape_errors_name_the_json_type(#[case] text: &str, #[case] json_type: &'static str) {
    let node: BetaNode = serde_json::from_str(text).unwrap();
    assert_eq!(
        validate_beta_matrix(node),
        Err(BetaShapeError::NotAnObject { json_type })
    );
}

#[rstest]
#[case::not_json("{not json")]
#[case::not_an_object("[]")]
#[tokio::test]
async fn corrupt_local_file(#[case] bundled: &'static str) {
    let (logs, _guard) = capture();
    let mut broken = loader(true, "");
    broken.bundled = bundled;
    let loaded = broken.matrix().await;
    assert_eq!(*loaded, matrix(EMPTY_SIX));
    let keys: Vec<&str> = loaded.0.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "anthropic",
            "azure_ai",
            "bedrock",
            "bedrock_converse",
            "vertex_ai",
            "provider_aliases"
        ]
    );
    assert_eq!(
        logs.events(),
        [
            (Level::ERROR, "local_load_failed".to_owned()),
            (Level::DEBUG, "cache_filled".to_owned()),
        ]
    );
}

#[tokio::test]
async fn remote_matrix_cache() {
    let (logs, _guard) = capture();
    let replies = vec![
        Reply::Answer(200, r#"{"anthropic":{"custom":"renamed"}}"#),
        Reply::Answer(200, r#"{"bedrock":{"custom":null}}"#),
    ];
    let mock = serve(replies, logs.clone()).await;
    let beta = loader(false, &mock.url);
    let first = beta.matrix().await;
    assert_eq!(*first, matrix(r#"{"anthropic":{"custom":"renamed"}}"#));
    assert_eq!(beta.matrix().await, first);
    assert_eq!(mock.request_count(), 1);
    assert_eq!(logs.events(), [(Level::DEBUG, "cache_filled".to_owned())]);

    let second = beta.reload().await;
    assert_eq!(*second, matrix(r#"{"bedrock":{"custom":null}}"#));
    assert_eq!(beta.matrix().await, second);
    assert_eq!(mock.request_count(), 2);
    let requests = mock.requests.lock().unwrap().clone();
    assert_eq!(
        requests,
        [
            ("GET /matrix.json HTTP/1.1".to_owned(), 0),
            ("GET /matrix.json HTTP/1.1".to_owned(), 2),
        ]
    );
    assert_eq!(
        logs.events(),
        [
            (Level::DEBUG, "cache_filled".to_owned()),
            (Level::INFO, "reloading".to_owned()),
            (Level::DEBUG, "cache_filled".to_owned()),
        ]
    );
}

#[rstest]
#[case::unavailable(503, "{}")]
#[case::not_found(404, r#"{"anthropic":{}}"#)]
#[case::redirect(301, "")]
#[case::invalid_json(200, "not json")]
#[case::empty_body(200, "")]
#[case::bare_object_start(200, "{")]
#[tokio::test]
async fn failed_fetch_falls_back_once(#[case] status: u16, #[case] body: &'static str) {
    let (logs, _guard) = capture();
    let mock = serve(vec![Reply::Answer(status, body)], logs.clone()).await;
    let beta = loader(false, &mock.url);
    let loaded = beta.matrix().await;
    assert_eq!(*loaded, matrix(BUNDLED_CONFIG));
    assert_eq!(beta.matrix().await, loaded);
    assert_eq!(mock.request_count(), 1);
    assert_eq!(warnings(&logs), ["fetch_failed"]);
}

#[tokio::test(start_paused = true)]
async fn fetch_timeout_falls_back() {
    let (logs, _guard) = capture();
    let mock = serve(vec![Reply::Stall], logs.clone()).await;
    let beta = loader(false, &mock.url);
    let started = Instant::now();
    let loaded = beta.matrix().await;
    let waited = started.elapsed();
    assert!(waited >= Duration::from_secs(5) && waited < Duration::from_secs(6));
    assert_eq!(*loaded, matrix(BUNDLED_CONFIG));
    assert_eq!(beta.matrix().await, loaded);
    assert_eq!(mock.request_count(), 1);
    assert_eq!(warnings(&logs), ["fetch_failed"]);
}

#[tokio::test(start_paused = true)]
async fn stalled_fetch_does_not_block_other_calls() {
    let (logs, _guard) = capture();
    let mock = serve(vec![Reply::Stall], logs).await;
    let beta = loader(false, &mock.url);
    let started = Instant::now();
    let (loaded, reloaded) = tokio::join!(beta.matrix(), beta.reload());
    assert!(started.elapsed() < Duration::from_secs(6));
    assert_eq!(*loaded, matrix(BUNDLED_CONFIG));
    assert_eq!(*reloaded, matrix(BUNDLED_CONFIG));
}

#[rstest]
#[case::empty_url("")]
#[case::not_a_url("not a url")]
#[case::unsupported_scheme("ftp://beta.test/matrix.json")]
#[tokio::test]
async fn unusable_url_falls_back(#[case] url: &str) {
    let (logs, _guard) = capture();
    let beta = loader(false, url);
    let loaded = beta.matrix().await;
    assert_eq!(*loaded, matrix(BUNDLED_CONFIG));
    assert_eq!(beta.matrix().await, loaded);
    assert_eq!(warnings(&logs), ["fetch_failed"]);
}

#[rstest]
#[case::at_the_limit(7, true)]
#[case::past_the_limit(6, false)]
fn decoder_checks_size_before_parsing(#[case] max_json_bytes: usize, #[case] accepted: bool) {
    let limits = ApiLimits {
        max_json_bytes,
        max_event_bytes: 1024,
        max_items: 1024,
    };
    let decoded = decode_beta_matrix(br#"{"a":1}"#, limits);
    if accepted {
        assert!(decoded.is_ok());
    } else {
        assert!(matches!(
            decoded,
            Err(BetaConfigError::TooLarge(ApiLimitError::JsonTooLarge {
                observed: 7,
                limit: 6
            }))
        ));
    }
}

#[test]
fn decoder_rejects_invalid_utf8() {
    assert!(matches!(
        decode_beta_matrix(&[0xff, 0xfe], ApiLimits::default()),
        Err(BetaConfigError::Utf8(_))
    ));
}

#[rstest]
#[case::empty("")]
#[case::open_brace("{")]
#[case::missing_value(r#"{"a":}"#)]
#[case::truncated_literal("nul")]
fn decoder_rejects_malformed_json(#[case] text: &str) {
    assert!(matches!(
        decode_beta_matrix(text.as_bytes(), ApiLimits::default()),
        Err(BetaConfigError::Json(_))
    ));
}

#[rstest]
#[case::first_stops_at_second(
    r#"{"anthropic":{},"provider_aliases":{"first":"second","second":"anthropic"}}"#,
    "first",
    "second"
)]
#[case::second_reaches_anthropic(
    r#"{"anthropic":{},"provider_aliases":{"first":"second","second":"anthropic"}}"#,
    "second",
    "anthropic"
)]
#[case::unknown_stays(
    r#"{"anthropic":{},"provider_aliases":{"first":"second","second":"anthropic"}}"#,
    "other",
    "other"
)]
#[case::bundled_has_no_aliases(BUNDLED_CONFIG, "vertex_ai_beta", "vertex_ai_beta")]
#[case::empty_alias_is_used(r#"{"anthropic":{},"provider_aliases":{"first":""}}"#, "first", "")]
#[case::alias_table_null(r#"{"anthropic":{},"provider_aliases":null}"#, "first", "first")]
#[case::alias_table_list(r#"{"anthropic":{},"provider_aliases":["first"]}"#, "first", "first")]
#[case::alias_number(r#"{"anthropic":{},"provider_aliases":{"first":5}}"#, "first", "first")]
#[case::alias_null(
    r#"{"anthropic":{},"provider_aliases":{"first":null}}"#,
    "first",
    "first"
)]
fn provider_alias_resolution(#[case] config: &str, #[case] provider: &str, #[case] expected: &str) {
    assert_eq!(matrix(config).provider_name(provider), expected);
}

#[rstest]
fn replays_corpus(#[files("fuzz/corpus/anthropic_beta_config/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    if let Ok(node) = decode_beta_matrix(&bytes, ApiLimits::default()) {
        drop(validate_beta_matrix(node));
    }
}
