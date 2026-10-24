#![cfg(test)]

use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::path::PathBuf;
use std::process;
use std::sync::Arc;
use std::thread;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::LiteLlmSettings;
use reqwest::{ClientBuilder, Method, Request, Url};
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tracing::subscriber::set_default;

use crate::error::ProxyError;
use crate::proxy::ProxiedClient;

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
    let settings = LiteLlmSettings::default();
    let policy = Arc::new(ProxyPolicy::resolve(&settings, &environment));
    let client = ProxiedClient::build(ClientBuilder::new(), &policy).unwrap();
    let url = Url::parse(&target.replace("LISTENER", &address)).unwrap();
    let request = Request::new(Method::GET, url);
    let mut observed = if expected.starts_with("error|") {
        format!("error|{}", client.execute(request).await.unwrap_err())
    } else {
        let proxy_side = async { Ok::<_, ProxyError>(answer(&listener).await) };
        let (_response, exchange) = tokio::try_join!(client.execute(request), proxy_side).unwrap();
        exchange
    };
    let written = fs::read_to_string(&log_path).unwrap();
    fs::remove_file(&log_path).unwrap();
    if written.contains("proxy setting ignored") {
        observed.push_str(" warned");
    }
    assert_eq!(observed, expected);
    let debug = format!("{client:?}");
    for leak in LEAKS {
        assert!(
            !written.contains(leak) && !debug.contains(leak),
            "{leak} leaked"
        );
    }
}

#[rstest]
fn replays_the_proxy_policy_corpus(#[files("fuzz/corpus/proxy_policy/*")] path: PathBuf) {
    let text = fs::read_to_string(path).unwrap();
    let (proxy, no_proxy) = text.split_once('\n').unwrap_or((&text, ""));
    let environment = BTreeMap::from([
        ("HTTPS_PROXY".to_owned(), proxy.to_owned()),
        ("NO_PROXY".to_owned(), no_proxy.to_owned()),
    ]);
    let settings = LiteLlmSettings::default();
    let policy = Arc::new(ProxyPolicy::resolve(&settings, &environment));
    let _route = policy.route(&Url::parse("https://api.example.com/v1").unwrap());
    ProxiedClient::build(ClientBuilder::new(), &policy).unwrap();
}
