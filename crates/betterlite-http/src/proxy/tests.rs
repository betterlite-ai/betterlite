#![cfg(test)]

use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::path::PathBuf;
use std::process;
use std::sync::Arc;
use std::thread;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{ClientBuilder, Method, Request, Url};
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tracing::subscriber::set_default;

use crate::error::HttpError;
use crate::passthrough::{
    buffer_response, construct_url_with_subpath, get_response_body, get_response_headers,
    streaming_response,
};
use crate::proxy::ProxiedClient;

// nothing listens on port 1, so a request that skips its proxy fails at once
const CLOSED: &str = "http://127.0.0.1:1/v1/models";

// past basic_auth the password lives only as base64 in Proxy-Authorization, so a leak can skip the plaintext
const LEAKS: [&str; 3] = ["secret", "dXNlcjpzZWNyZXQ=", "dXNAZXI6cEBzczpzZWNyZXQ="];

#[test]
fn passthrough_url_joining_behavior() {
    assert_eq!(
        construct_url_with_subpath("https://upstream/base", "v1/../items/", true),
        "https://upstream/base/items/"
    );
}

#[test]
fn passthrough_response_headers_behavior() {
    let mut headers = HeaderMap::new();
    headers.insert("content-length", HeaderValue::from_static("10"));
    let output = get_response_headers(&headers, Some("call-1"), Some(&headers));
    assert!(!output.contains_key("content-length"));
    assert_eq!(output.get("x-litellm-call-id").unwrap(), "call-1");
    assert_eq!(
        get_response_body(br#"{"ok":true}"#).map(|value| value.to_string()),
        Some(r#"{"ok":true}"#.to_owned())
    );
}

#[test]
fn passthrough_buffering_behavior() {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    assert!(buffer_response(200, &headers));
    headers.insert(
        "content-type",
        HeaderValue::from_static("application/octet-stream"),
    );
    assert!(!buffer_response(200, &headers));
    assert!(!streaming_response(&headers));
    headers.insert(
        "content-type",
        HeaderValue::from_static("text/event-stream"),
    );
    assert!(streaming_response(&headers));
}

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
