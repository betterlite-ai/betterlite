#![cfg(test)]

use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::path::PathBuf;
use std::process;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use reqwest::{ClientBuilder, Method, Request, Url};
use rstest::rstest;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::sleep;
use tracing::subscriber::set_default;

use crate::error::HttpError;
use crate::proxy::{JsonPost, ProxiedClient};

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
    let policy = Arc::new(ProxyPolicy::resolve(&trust, &environment));
    let client = ProxiedClient::build(ClientBuilder::new(), &policy).unwrap();
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
    let policy = Arc::new(ProxyPolicy::resolve(
        &ProxyTrust::default(),
        &BTreeMap::new(),
    ));
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
    for line in [
        "POST /v1/chat/completions HTTP/1.1",
        "authorization: Bearer sk-test",
        "openai-organization: org-1",
        "content-type: application/json",
        "accept: application/json",
    ] {
        assert!(sent.contains(line), "{line} missing from {sent}");
    }
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
    let policy = Arc::new(ProxyPolicy::resolve(
        &ProxyTrust::default(),
        &BTreeMap::new(),
    ));
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

#[rstest]
fn replays_the_proxy_policy_corpus(#[files("fuzz/corpus/proxy_policy/*")] path: PathBuf) {
    let text = fs::read_to_string(path).unwrap();
    let (proxy, no_proxy) = text.split_once('\n').unwrap_or((&text, ""));
    let environment = BTreeMap::from([
        ("HTTPS_PROXY".to_owned(), proxy.to_owned()),
        ("NO_PROXY".to_owned(), no_proxy.to_owned()),
    ]);
    let trust = ProxyTrust::default();
    let policy = Arc::new(ProxyPolicy::resolve(&trust, &environment));
    let _route = policy.route(&Url::parse("https://api.example.com/v1").unwrap());
    ProxiedClient::build(ClientBuilder::new(), &policy).unwrap();
}
