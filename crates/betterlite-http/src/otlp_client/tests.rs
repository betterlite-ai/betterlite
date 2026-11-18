#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http_fixture::server::serve;
use bytes::Bytes;
use futures_util::FutureExt as _;
use opentelemetry_http::{HttpClient as _, Request};
use rstest::rstest;
use rustls::crypto::aws_lc_rs;
use rustls::{ClientConfig, RootCertStore};

use super::OtlpClient;

fn client(variables: &[(&str, &str)]) -> OtlpClient {
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let policy = Arc::new(ProxyPolicy::resolve(&ProxyTrust::default(), &environment).unwrap());
    let tls = ClientConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(RootCertStore::empty())
        .with_no_client_auth();
    OtlpClient::with_tls(tls, &policy).unwrap()
}

#[rstest]
#[case::authenticated_proxy(&[("HTTP_PROXY", "http://user:secret@LISTENER")], "http://127.0.0.1:1/v1/metrics", "POST http://127.0.0.1:1/v1/metrics HTTP/1.1", Some("Basic dXNlcjpzZWNyZXQ="))]
#[case::anonymous_proxy(&[("HTTP_PROXY", "http://LISTENER")], "http://127.0.0.1:1/v1/metrics", "POST http://127.0.0.1:1/v1/metrics HTTP/1.1", None)]
#[case::encoded_credentials(&[("HTTP_PROXY", "http://us%40er:p%40ss%3Asecret@LISTENER")], "http://127.0.0.1:1/v1/metrics", "POST http://127.0.0.1:1/v1/metrics HTTP/1.1", Some("Basic dXNAZXI6cEBzczpzZWNyZXQ="))]
#[case::bypass(&[("HTTP_PROXY", "http://user:secret@127.0.0.1:1"), ("NO_PROXY", "127.0.0.1")], "http://LISTENER/v1/metrics", "POST /v1/metrics HTTP/1.1", None)]
#[case::primary_before_all(&[("HTTP_PROXY", "http://LISTENER"), ("ALL_PROXY", "http://127.0.0.1:1")], "http://127.0.0.1:1/v1/metrics", "POST http://127.0.0.1:1/v1/metrics HTTP/1.1", None)]
#[case::fallback(&[("HTTP_PROXY", "http://LISTENER"), ("DISABLE_AIOHTTP_TRUST_ENV", "true"), ("AIOHTTP_TRUST_ENV", "true")], "http://127.0.0.1:1/v1/metrics", "POST http://127.0.0.1:1/v1/metrics HTTP/1.1", None)]
fn applies_proxy_policy(
    #[case] variables: &[(&str, &str)],
    #[case] endpoint: &str,
    #[case] request_line: &str,
    #[case] authorization: Option<&str>,
) {
    let serving = serve(
        b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\nx-collector: yes\r\n\r\nOK"
            .to_vec(),
        Duration::ZERO,
    )
    .unwrap();
    let address = serving.address.to_string();
    let configured: Vec<(&str, String)> = variables
        .iter()
        .map(|(name, value)| (*name, value.replace("LISTENER", &address)))
        .collect();
    let pairs: Vec<(&str, &str)> = configured
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let client = client(&pairs);
    assert!(!format!("{client:?}").contains("secret"));
    let request = Request::post(endpoint.replace("LISTENER", &address))
        .body(Bytes::new())
        .unwrap();
    let response = client.send_bytes(request).now_or_never().unwrap().unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["x-collector"], "yes");
    assert_eq!(response.body(), "OK");
    let text = serving.finish().unwrap();
    assert!(text.line.starts_with(request_line), "{}", text.line);
    assert_eq!(
        text.head
            .lines()
            .find_map(|line| line.strip_prefix("proxy-authorization: ")),
        authorization
    );
}

#[rstest]
#[case::unsupported(
    "socks5://user:secret@localhost:1",
    "only http:// proxies are supported"
)]
#[case::invalid("http://user:secret@localhost:99999", "proxy URL is not valid")]
fn rejects_invalid_proxy_routes(#[case] proxy: &str, #[case] message: &str) {
    let client = client(&[("HTTP_PROXY", proxy)]);
    let request = Request::post("http://127.0.0.1:1/v1/metrics")
        .body(Bytes::new())
        .unwrap();
    let error = client
        .send_bytes(request)
        .now_or_never()
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains(message), "{error}");
    assert!(!format!("{error:?}").contains("secret"));
}

#[test]
fn rejects_oversized_response_before_buffering() {
    let serving = serve(
        b"HTTP/1.1 200 OK\r\ncontent-length: 4194305\r\nconnection: close\r\n\r\n".to_vec(),
        Duration::ZERO,
    )
    .unwrap();
    let client = client(&[]);
    let request = Request::post(format!("http://{}/v1/metrics", serving.address))
        .body(Bytes::new())
        .unwrap();
    let error = client
        .send_bytes(request)
        .now_or_never()
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("4 MiB"), "{error}");
    serving.finish().unwrap();
}

#[rstest]
#[case::at_limit(0x0040_0000, "", true)]
#[case::over_limit(0x0040_0001, "", false)]
#[case::declared_at_limit(0x0040_0000, "content-length: 4194304\r\n", true)]
fn bounds_collector_responses(#[case] length: usize, #[case] header: &str, #[case] accepted: bool) {
    let mut answer = format!("HTTP/1.1 200 OK\r\n{header}connection: close\r\n\r\n").into_bytes();
    answer.extend(vec![b'x'; length]);
    let serving = serve(answer, Duration::ZERO).unwrap();
    let client = client(&[]);
    let request = Request::post(format!("http://{}/v1/metrics", serving.address))
        .body(Bytes::new())
        .unwrap();
    let result = client.send_bytes(request).now_or_never().unwrap();
    if accepted {
        assert_eq!(result.unwrap().body().len(), length);
    } else {
        assert!(result.unwrap_err().to_string().contains("4 MiB"));
    }
    serving.finish().unwrap();
}
