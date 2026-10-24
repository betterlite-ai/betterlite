#![cfg(test)]

use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::path::Path;
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

use crate::proxy::ProxiedClient;

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
#[case::forwards_absolute_form("http://us%40er:p%40ss%3Asecret@LISTENER", "", "http://api.example.com/v1/models", "GET http://api.example.com/v1/models HTTP/1.1|Basic dXNAZXI6cEBzczpzZWNyZXQ=")]
#[case::forwards_without_credentials("LISTENER", "", "http://api.example.com/v1/models", "GET http://api.example.com/v1/models HTTP/1.1|none")]
#[case::bypassed_host_goes_direct("http://user:secret@proxy.invalid:9", "127.0.0.1", "http://LISTENER/v1/models", "GET /v1/models HTTP/1.1|none")]
#[case::unsupported_scheme_fails_by_name("socks5://user:secret@LISTENER", "", "http://api.example.com/v1/models", "error|the proxy for this request uses socks5://, and only http:// proxies are supported")]
#[case::invalid_url_fails_by_name("http://user:secret@LISTENER:99999", "", "http://api.example.com/v1/models", "error|the http proxy URL is not valid")]
#[case::dropped_proxy_warns_without_password("https://user:secret@proxy.invalid:9", "", "http://LISTENER/v1/models", "GET /v1/models HTTP/1.1|none")]
#[tokio::test]
async fn http_proxies_proxy_policy_tests_behavior(
    #[case] proxy_url: &str,
    #[case] no_proxy: &str,
    #[case] target: &str,
    #[case] expected: &str,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let mut environment = BTreeMap::new();
    environment.insert("HTTP_PROXY".to_owned(), proxy_url.replace("LISTENER", &address));
    environment.insert("NO_PROXY".to_owned(), no_proxy.to_owned());
    let mut settings = LiteLlmSettings::default();
    settings.aiohttp_trust_env = proxy_url.starts_with("https");
    settings.disable_aiohttp_trust_env = settings.aiohttp_trust_env;
    let case = thread::current().name().unwrap().replace("::", "-");
    let log_path = env::temp_dir().join(format!("betterlite-proxy-{}-{case}", process::id()));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(Arc::new(File::create(&log_path).unwrap()))
        .finish();
    let _guard = set_default(subscriber);
    let policy = Arc::new(ProxyPolicy::resolve(&settings, &environment));
    let client = ProxiedClient::build(ClientBuilder::new(), &policy).unwrap();
    let url = Url::parse(&target.replace("LISTENER", &address)).unwrap();
    let request = Request::new(Method::GET, url);
    let observed = if expected.starts_with("error|") {
        format!("error|{}", client.execute(request).await.unwrap_err())
    } else {
        let (response, exchange) = tokio::join!(client.execute(request), answer(&listener));
        response.unwrap();
        exchange
    };
    assert_eq!(observed, expected);
    let written = fs::read_to_string(&log_path).unwrap();
    fs::remove_file(&log_path).unwrap();
    assert!(!written.contains("secret"));
    assert!(!format!("{client:?}").contains("secret"));
    let warned = written.contains("proxy setting ignored");
    assert_eq!(warned, proxy_url.starts_with("https"));
}

#[test]
fn replays_the_proxy_policy_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/proxy_policy");
    for entry in fs::read_dir(corpus).unwrap() {
        let text = fs::read_to_string(entry.unwrap().path()).unwrap();
        let (proxy, no_proxy) = text.split_once('\n').unwrap_or((&text, ""));
        let mut environment = BTreeMap::new();
        environment.insert("HTTPS_PROXY".to_owned(), proxy.to_owned());
        environment.insert("NO_PROXY".to_owned(), no_proxy.to_owned());
        let policy = Arc::new(ProxyPolicy::resolve(&LiteLlmSettings::default(), &environment));
        let _route = policy.route(&Url::parse("https://api.example.com/v1").unwrap());
        ProxiedClient::build(ClientBuilder::new(), &policy).unwrap();
    }
}
