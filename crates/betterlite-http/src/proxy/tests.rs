#![cfg(test)]

use std::collections::BTreeMap;
use std::env;
use std::fmt::Debug;
use std::fs::{self, File};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use futures_util::{StreamExt as _, stream};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use reqwest::{ClientBuilder, Method, Request, Url};
use rstest::rstest;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpSocket, TcpStream as Connection};
use tokio::task::JoinSet;
use tokio::time::sleep;
use tracing::subscriber::set_default;

use crate::error::HttpError;
use crate::proxy::limits::{HTTP1_PARSER_CEILING, TransportLimits};
use crate::proxy::outbound::{OutboundRequest, RequestBody};
use crate::proxy::{JsonPost, ProxiedClient, base_builder};

// nothing listens on port 1, so a request that skips its proxy fails at once
const CLOSED: &str = "http://127.0.0.1:1/v1/models";

// past basic_auth the password lives only as base64 in Proxy-Authorization, so a leak can skip the plaintext
const LEAKS: [&str; 3] = ["secret", "dXNlcjpzZWNyZXQ=", "dXNAZXI6cEBzczpzZWNyZXQ="];

async fn answer(listener: &TcpListener) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        head.push(stream.read_u8().await.unwrap());
    }
    let reply = b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
    stream.write_all(reply).await.unwrap();
    let text = String::from_utf8(head).unwrap();
    let request_line = text.lines().next().unwrap().to_owned();
    let authorization = text
        .lines()
        .find_map(|line| line.strip_prefix("proxy-authorization: "))
        .unwrap_or("none");
    format!("{request_line}|{authorization}")
}

#[rstest]
#[case::forwards_absolute_form(&[("HTTP_PROXY", "http://us%40er:p%40ss%3Asecret@LISTENER")], CLOSED, "GET http://127.0.0.1:1/v1/models HTTP/1.1|Basic dXNAZXI6cEBzczpzZWNyZXQ=")]
#[case::forwards_without_credentials(&[("HTTP_PROXY", "LISTENER")], CLOSED, "GET http://127.0.0.1:1/v1/models HTTP/1.1|none")]
#[case::scheme_proxy_beats_all_proxy(&[("ALL_PROXY", "http://127.0.0.1:1"), ("HTTP_PROXY", "LISTENER")], CLOSED, "GET http://127.0.0.1:1/v1/models HTTP/1.1|none")]
#[case::bypassed_host_goes_direct(&[("HTTP_PROXY", "http://user:secret@127.0.0.1:1"), ("NO_PROXY", "127.0.0.1")], "http://LISTENER/v1/models", "GET /v1/models HTTP/1.1|none")]
#[case::unsupported_scheme_fails_by_name(&[("HTTP_PROXY", "socks5://user:secret@LISTENER")], CLOSED, "error|the proxy for this request uses socks5://, and only http:// proxies are supported")]
#[case::invalid_url_fails_by_name(&[("HTTP_PROXY", "http://user:secret@LISTENER:99999")], CLOSED, "error|the http proxy URL is not valid")]
#[case::dropped_proxy_warns_without_password(&[("AIOHTTP_TRUST_ENV", "true"), ("DISABLE_AIOHTTP_TRUST_ENV", "true"), ("HTTP_PROXY", "https://user:secret@127.0.0.1:1")], "http://LISTENER/v1/models", "GET /v1/models HTTP/1.1|none warned")]
#[tokio::test]
async fn http_proxies_proxy_policy_tests_behavior(
    #[case] variables: &[(&str, &str)],
    #[case] target: &str,
    #[case] expected: &str,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let environment = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.replace("LISTENER", &address)))
        .collect();
    let case = thread::current().name().unwrap().replace("::", "-");
    let log_path = env::temp_dir().join(format!("betterlite-proxy-{}-{case}", process::id()));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(Arc::new(File::create(&log_path).unwrap()))
        .finish();
    let _guard = set_default(subscriber);
    let trust = ProxyTrust::default();
    let policy = Arc::new(ProxyPolicy::resolve(&trust, &environment).unwrap());
    let client =
        ProxiedClient::build(ClientBuilder::new(), &policy, TransportLimits::DEFAULT).unwrap();
    let url = Url::parse(&target.replace("LISTENER", &address)).unwrap();
    let request = Request::new(Method::GET, url);
    let (mut observed, error_debug) = if expected.starts_with("error|") {
        let error = client.execute(request).await.unwrap_err();
        (format!("error|{error}"), format!("{error:?}"))
    } else {
        let proxy_side = async { Ok::<_, HttpError>(answer(&listener).await) };
        let (_response, exchange) = tokio::try_join!(client.execute(request), proxy_side).unwrap();
        (exchange, String::new())
    };
    let debug = format!("{client:?}{error_debug}");
    let written = fs::read_to_string(&log_path).unwrap();
    fs::remove_file(&log_path).unwrap();
    if written.contains("proxy setting ignored") {
        observed.push_str(" warned");
    }
    assert_eq!(observed, expected);
    let leaked: Vec<&str> = LEAKS
        .into_iter()
        .filter(|leak| written.contains(leak) || debug.contains(leak))
        .collect();
    assert_eq!(leaked, Vec::<&str>::new());
}

async fn reply_after(listener: TcpListener, delay: Duration) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut received = Vec::new();
    while !received.ends_with(b"\r\n\r\n{}") {
        received.push(stream.read_u8().await.unwrap());
    }
    sleep(delay).await;
    let reply = b"HTTP/1.1 201 Created\r\nx-request-id: r1\r\ncontent-length: 5\r\nconnection: close\r\n\r\nhello";
    let _written = stream.write_all(reply).await;
    String::from_utf8(received).unwrap()
}

#[rstest]
#[case::body_at_the_limit(5, Duration::ZERO, "201 hello")]
#[case::body_over_the_limit(4, Duration::ZERO, "the response body is larger than 4 bytes")]
#[case::slower_than_the_timeout(5, Duration::from_secs(2), "the request timed out")]
#[tokio::test]
async fn posts_json_with_a_bearer_key_and_bounded_answer(
    #[case] body_limit: usize,
    #[case] delay: Duration,
    #[case] expected: &str,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let client = ProxiedClient::outbound(&policy).unwrap();
    let key = SecretString::from("sk-test");
    let post = JsonPost {
        url: Url::parse(&format!("http://{address}/v1/chat/completions")).unwrap(),
        bearer: &key,
        headers: vec![("OpenAI-Organization", "org-1".to_owned())],
        body: b"{}".to_vec(),
        timeout: Duration::from_millis(300),
        body_limit,
    };
    let (sent, answered) = tokio::join!(reply_after(listener, delay), client.post_json(post));
    assert!(sent.contains("POST /v1/chat/completions HTTP/1.1"));
    assert!(sent.contains("authorization: Bearer sk-test"));
    assert!(sent.contains("openai-organization: org-1"));
    assert!(sent.contains("content-type: application/json"));
    assert!(sent.contains("accept: application/json"));
    let observed = answered.map_or_else(
        |error| error.to_string(),
        |response| {
            assert_eq!(response.headers.get("x-request-id").unwrap(), "r1");
            format!(
                "{} {}",
                response.status,
                String::from_utf8(response.body).unwrap()
            )
        },
    );
    assert_eq!(observed, expected);
}

#[tokio::test]
async fn reports_a_refused_connection_as_a_failed_request() {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let client = ProxiedClient::outbound(&policy).unwrap();
    let key = SecretString::from("sk-test");
    let post = JsonPost {
        url: Url::parse(CLOSED).unwrap(),
        bearer: &key,
        headers: Vec::new(),
        body: Vec::new(),
        timeout: Duration::from_secs(5),
        body_limit: 1,
    };
    let error = client.post_json(post).await.unwrap_err();
    assert!(matches!(error, HttpError::Request(_)), "{error:?}");
}

pub(crate) struct Server {
    pub(crate) base: Url,
    pub(crate) heads: Arc<Mutex<Vec<String>>>,
    pub(crate) connections: Arc<AtomicUsize>,
    _listener: Option<TcpListener>,
    _fillers: Vec<TcpStream>,
    _tasks: JoinSet<()>,
}

pub(crate) fn reply(head: &[&str], body: &[u8]) -> Vec<u8> {
    let mut bytes = format!("HTTP/1.1 {}\r\n\r\n", head.join("\r\n")).into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn header_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|start| start.saturating_add(4))
}

async fn fill(stream: &mut Connection, buffer: &mut Vec<u8>) -> bool {
    let mut chunk = [0_u8; 4096];
    let read = stream.read(&mut chunk).await.unwrap_or(0);
    buffer.extend_from_slice(chunk.get(..read).unwrap_or_default());
    read > 0
}

async fn read_request(stream: &mut Connection, buffer: &mut Vec<u8>) -> Option<String> {
    let end = loop {
        if let Some(end) = header_end(buffer) {
            break end;
        }
        if !fill(stream, buffer).await {
            return None;
        }
    };
    let head = String::from_utf8(buffer.get(..end)?.to_vec()).ok()?;
    let length: usize = head
        .to_ascii_lowercase()
        .lines()
        .find_map(|line| line.strip_prefix("content-length: ")?.parse().ok())
        .unwrap_or(0);
    let total = end.saturating_add(length);
    while buffer.len() < total {
        if !fill(stream, buffer).await {
            return None;
        }
    }
    let body = String::from_utf8(buffer.get(end..total)?.to_vec()).ok()?;
    buffer.drain(..total);
    Some(format!("{head}{body}"))
}

async fn converse(
    mut stream: Connection,
    heads: Arc<Mutex<Vec<String>>>,
    answer: Vec<u8>,
    delay: Duration,
) {
    let closes = str::from_utf8(&answer).is_ok_and(|text| text.contains("connection: close"));
    let mut buffer = Vec::new();
    while let Some(request) = read_request(&mut stream, &mut buffer).await {
        heads.lock().unwrap().push(request);
        sleep(delay).await;
        if answer.is_empty() {
            return;
        }
        let _written = stream.write_all(&answer).await;
        if closes {
            return;
        }
    }
}

async fn accept_loop(
    listener: TcpListener,
    heads: Arc<Mutex<Vec<String>>>,
    connections: Arc<AtomicUsize>,
    answer: Vec<u8>,
    delay: Duration,
) {
    let mut open = JoinSet::new();
    while let Ok((stream, _)) = listener.accept().await {
        connections.fetch_add(1, Ordering::SeqCst);
        open.spawn(converse(stream, Arc::clone(&heads), answer.clone(), delay));
    }
}

// Answers every request on a connection with `answer` after `delay`; an empty answer closes the connection unanswered.
pub(crate) async fn serve(answer: Vec<u8>, delay: Duration) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let heads: Arc<Mutex<Vec<String>>> = Arc::default();
    let connections = Arc::new(AtomicUsize::new(0));
    let mut tasks = JoinSet::new();
    tasks.spawn(accept_loop(
        listener,
        Arc::clone(&heads),
        Arc::clone(&connections),
        answer,
        delay,
    ));
    Server {
        base,
        heads,
        connections,
        _listener: None,
        _fillers: Vec::new(),
        _tasks: tasks,
    }
}

// A listener whose accept queue is full, so a further connect hangs instead of failing.
fn saturated() -> Server {
    let socket = TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let listener = socket.listen(1).unwrap();
    let address = listener.local_addr().unwrap();
    let fillers = (0_u8..4)
        .filter_map(|_| TcpStream::connect_timeout(&address, Duration::from_millis(100)).ok())
        .collect();
    Server {
        base: Url::parse(&format!("http://{address}/")).unwrap(),
        heads: Arc::default(),
        connections: Arc::default(),
        _listener: Some(listener),
        _fillers: fillers,
        _tasks: JoinSet::new(),
    }
}

pub(crate) fn kind<T>(value: &T) -> String
where
    T: Debug + ?Sized,
{
    format!("{value:?}")
        .chars()
        .take_while(|letter| letter.is_alphanumeric())
        .collect()
}

pub(crate) fn default_client() -> ProxiedClient {
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    ProxiedClient::build(base_builder(), &policy, TransportLimits::DEFAULT).unwrap()
}

fn big_header(size: usize) -> String {
    format!("x-big: {}", "a".repeat(size))
}

async fn exchange(client: &ProxiedClient, base: &Url, index: usize, deadline: Duration) -> String {
    let mut headers = HeaderMap::new();
    let bearer = HeaderValue::try_from(format!("Bearer key-{index}")).unwrap();
    headers.insert(AUTHORIZATION, bearer);
    let request = OutboundRequest {
        method: Method::POST,
        url: base.clone(),
        query: Vec::new(),
        headers,
        body: RequestBody::Json(b"{}".to_vec()),
        deadline,
    };
    match client.send_buffered(request).await {
        Ok(response) => format!(
            "{}:{}",
            response.status,
            String::from_utf8(response.body).unwrap()
        ),
        Err(error) => kind(&error),
    }
}

struct Run {
    requests: usize,
    limits: TransportLimits,
    deadline: Duration,
}

const ONE: Run = Run {
    requests: 1,
    limits: TransportLimits::DEFAULT,
    deadline: Duration::from_secs(5),
};

#[rstest]
#[case::reuses_one_connection_without_sharing_credentials_or_cookies(Some(reply(&["200 OK", "content-length: 2", "set-cookie: sid=1"], b"ok")), Duration::ZERO, Run { requests: 3, ..ONE }, "200:ok 200:ok 200:ok conn=1")]
#[case::body_at_the_limit(Some(reply(&["200 OK", "content-length: 5"], b"hello")), Duration::ZERO, Run { limits: TransportLimits { body_bytes: 5, ..TransportLimits::DEFAULT }, ..ONE }, "200:hello conn=1")]
#[case::declared_length_over_the_limit(Some(reply(&["200 OK", "content-length: 6"], b"hellos")), Duration::ZERO, Run { limits: TransportLimits { body_bytes: 5, ..TransportLimits::DEFAULT }, ..ONE }, "BodyTooLarge conn=1")]
#[case::chunked_body_over_the_limit(Some(reply(&["200 OK", "transfer-encoding: chunked"], b"3\r\nabc\r\n3\r\ndef\r\n0\r\n\r\n")), Duration::ZERO, Run { limits: TransportLimits { body_bytes: 5, ..TransportLimits::DEFAULT }, ..ONE }, "BodyTooLarge conn=1")]
#[case::error_body_over_the_limit(Some(reply(&["500 Error", "content-length: 6"], b"hellos")), Duration::ZERO, Run { limits: TransportLimits { body_bytes: 5, ..TransportLimits::DEFAULT }, ..ONE }, "BodyTooLarge conn=1")]
#[case::headers_at_the_limit(Some(reply(&["200 OK", "content-length: 2", "x-a: 1234567890"], b"ok")), Duration::ZERO, Run { limits: TransportLimits { header_bytes: 28, ..TransportLimits::DEFAULT }, ..ONE }, "200:ok conn=1")]
#[case::headers_over_the_limit(Some(reply(&["200 OK", "content-length: 2", "x-a: 1234567890"], b"ok")), Duration::ZERO, Run { limits: TransportLimits { header_bytes: 27, ..TransportLimits::DEFAULT }, ..ONE }, "HeadersTooLarge conn=1")]
#[case::headers_over_the_limit_below_the_parser_ceiling(Some(reply(&["200 OK", "content-length: 2", &big_header(HTTP1_PARSER_CEILING - 20_000)], b"ok")), Duration::ZERO, ONE, "HeadersTooLarge conn=1")]
#[case::headers_past_the_parser_ceiling(Some(reply(&["200 OK", "content-length: 2", &big_header(HTTP1_PARSER_CEILING * 2)], b"ok")), Duration::ZERO, ONE, "Request conn=1")]
#[case::body_shorter_than_its_length(Some(reply(&["200 OK", "content-length: 10", "connection: close"], b"abc")), Duration::ZERO, ONE, "Body conn=1")]
#[case::conflicting_lengths(Some(reply(&["200 OK", "content-length: 2", "content-length: 3"], b"ok")), Duration::ZERO, ONE, "Request conn=1")]
#[case::malformed_chunk_encoding(Some(reply(&["200 OK", "transfer-encoding: chunked", "connection: close"], b"zz\r\nabc")), Duration::ZERO, ONE, "Body conn=1")]
#[case::total_deadline(Some(reply(&["200 OK", "content-length: 2"], b"ok")), Duration::from_secs(2), Run { deadline: Duration::from_millis(300), ..ONE }, "Deadline conn=1")]
#[case::read_timeout_on_a_stalled_body(Some(reply(&["200 OK", "content-length: 10"], b"abc")), Duration::ZERO, Run { limits: TransportLimits { read_timeout: Duration::from_millis(300), ..TransportLimits::DEFAULT }, ..ONE }, "Deadline conn=1")]
#[case::connect_timeout(None, Duration::ZERO, Run { limits: TransportLimits { connect_timeout: Duration::from_millis(300), ..TransportLimits::DEFAULT }, ..ONE }, "ConnectTimeout conn=0")]
#[case::no_retry_after_a_dropped_connection(
    Some(Vec::new()),
    Duration::ZERO,
    ONE,
    "Request conn=1"
)]
#[tokio::test]
async fn http_pool_tests_behavior(
    #[case] scripted: Option<Vec<u8>>,
    #[case] delay: Duration,
    #[case] run: Run,
    #[case] expected: &str,
) {
    let server = match scripted {
        Some(bytes) => serve(bytes, delay).await,
        None => saturated(),
    };
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap());
    let client = ProxiedClient::build(base_builder(), &policy, run.limits).unwrap();
    let observed: Vec<String> = stream::iter(0..run.requests)
        .then(|index| exchange(&client, &server.base, index, run.deadline))
        .collect()
        .await;
    let summary = format!(
        "{} conn={}",
        observed.join(" "),
        server.connections.load(Ordering::SeqCst)
    );
    assert_eq!(summary, expected);
    let heads = server.heads.lock().unwrap().clone();
    let malformed: Vec<&String> = heads
        .iter()
        .enumerate()
        .filter(|(index, head)| {
            let lower = head.to_ascii_lowercase();
            !head.contains(&format!("Bearer key-{index}"))
                || lower.matches("authorization:").count() != 1
                || lower.contains("\r\ncookie:")
        })
        .map(|(_, head)| head)
        .collect();
    assert_eq!(malformed, Vec::<&String>::new());
}

#[rstest]
fn replays_the_proxy_policy_corpus(#[files("fuzz/corpus/proxy_policy/*")] path: PathBuf) {
    let text = fs::read_to_string(path).unwrap();
    let (proxy, no_proxy) = text.split_once('\n').unwrap_or((&text, ""));
    let environment = BTreeMap::from([
        ("HTTPS_PROXY".to_owned(), proxy.to_owned()),
        ("NO_PROXY".to_owned(), no_proxy.to_owned()),
    ]);
    let trust = ProxyTrust::default();
    let policy = Arc::new(ProxyPolicy::resolve(&trust, &environment).unwrap());
    let _route = policy.route(&Url::parse("https://api.example.com/v1").unwrap());
    ProxiedClient::build(ClientBuilder::new(), &policy, TransportLimits::DEFAULT).unwrap();
}
