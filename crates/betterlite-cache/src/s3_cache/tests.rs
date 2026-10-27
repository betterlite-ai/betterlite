#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::num::NonZeroUsize;
use std::path::Path;
use std::time::{Duration, SystemTime};

use aws_sdk_s3::config::SharedHttpClient;
use aws_smithy_http_client::tls::{Provider, rustls_provider::CryptoMode};
use betterlite_config::loader::ConfigLoader;
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::time::timeout;

use super::body::decode_body;
use super::entry::CachedEntry;
use super::{Disconnect as _, S3Cache};

#[test]
fn replays_the_s3_cache_body_corpus() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/s3_cache_body");
    let results: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let bytes = fs::read(entry.unwrap().path()).unwrap();
            decode_body(&bytes)
        })
        .collect();
    assert!(!results.is_empty());
}

fn configured_cache(listener: &TcpListener, prefix: &str, limit: usize) -> S3Cache {
    let address = listener.local_addr().unwrap();
    let yaml = format!(
        "litellm_settings:\n  cache_params:\n    s3_bucket_name: responses\n    s3_region_name: us-east-1\n    s3_endpoint_url: http://{address}\n    s3_path: '{prefix}'\n    s3_aws_access_key_id: test-key\n    s3_aws_secret_access_key: test-secret\n    s3_config:\n      read_timeout: 1\n      retries:\n        max_attempts: 0\n      s3:\n        addressing_style: path\n"
    );
    cache_from(&yaml, limit, Duration::from_secs(5))
}

fn cache_from(yaml: &str, limit: usize, operation_timeout: Duration) -> S3Cache {
    let config = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load(yaml)
    .unwrap();
    let transport = aws_smithy_http_client::Builder::new()
        .tls_provider(Provider::Rustls(CryptoMode::AwsLc))
        .build_https();
    S3Cache::new(
        &config.sections.s3_response_cache.unwrap(),
        SharedHttpClient::new(transport),
        || {
            SystemTime::UNIX_EPOCH
                .checked_add(Duration::from_secs(1_000))
                .unwrap()
        },
        NonZeroUsize::new(limit).unwrap(),
        operation_timeout,
    )
}

async fn serve(listener: &TcpListener, response: &[u8]) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0; 4096];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() < 0x0001_0000);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    stream.write_all(response).await.unwrap();
    String::from_utf8(bytes).unwrap()
}

#[rstest]
#[case::empty("", "/responses/response/key")]
#[case::slash("/", "/responses//response/key")]
#[case::prefix("prefix", "/responses/prefix/response/key")]
#[case::trailing("prefix///", "/responses/prefix/response/key")]
#[tokio::test]
async fn put_key_and_headers(#[case] prefix: &str, #[case] target: &str) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = configured_cache(&listener, prefix, 4096);
    let value = decode_body(br#"{"ok":true}"#).unwrap();
    let operation = async {
        let ((), request) = tokio::join!(
            cache.async_set_cache("response:key", &value, Some(60)),
            serve(
                &listener,
                b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
        );
        let headers = request.to_lowercase();
        assert!(
            request.starts_with(&format!("PUT {target}?x-id=PutObject HTTP/1.1\r\n")),
            "{request}"
        );
        assert!(headers.contains("cache-control: immutable, max-age=60, s-maxage=60\r\n"));
        assert!(headers.contains("expires: thu, 01 jan 1970 00:17:40 gmt\r\n"));
        assert!(headers.contains("content-type: application/json\r\n"));
        assert!(headers.contains("content-language: en\r\n"));
        assert!(headers.contains("content-disposition: inline; filename=\""));
        assert!(headers.contains("response/key.json\"\r\n"));
    };
    timeout(Duration::from_secs(5), operation).await.unwrap();
}

#[tokio::test]
async fn no_ttl_omits_expiry() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = configured_cache(&listener, "", 4096);
    let value = decode_body(b"{}").unwrap();
    let operation = async {
        let ((), request) = tokio::join!(
            cache.async_set_cache("key", &value, None),
            serve(
                &listener,
                b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
        );
        assert!(request.contains("immutable, max-age=31536000, s-maxage=31536000"));
        assert!(!request.to_lowercase().contains("expires:"));
    };
    timeout(Duration::from_secs(5), operation).await.unwrap();
}

#[rstest]
#[case::present("", br#"{"ok":true}"#, true)]
#[case::expired("Expires: Thu, 01 Jan 1970 00:16:39 GMT\r\n", b"{}", false)]
#[case::exact_expiry("Expires: Thu, 01 Jan 1970 00:16:40 GMT\r\n", b"{}", true)]
#[case::future("Expires: Thu, 01 Jan 1970 00:16:41 GMT\r\n", b"{}", true)]
#[case::invalid_expiry("Expires: invalid\r\n", b"{}", true)]
#[case::iso_expired("Expires: 1970-01-01T00:16:39Z\r\n", b"{}", false)]
#[case::iso_offset_future("Expires: 1970-01-01T01:16:41+01:00\r\n", b"{}", true)]
#[case::epoch_expired("Expires: 999\r\n", b"{}", false)]
#[case::epoch_future("Expires: 1001\r\n", b"{}", true)]
#[case::malformed("", b"not JSON", false)]
#[case::utf8("", b"\xff", false)]
#[case::oversize("", b"{\"response\":\"over the configured size limit\"}", false)]
#[tokio::test]
async fn get_outcomes(#[case] headers: &str, #[case] body: &[u8], #[case] hit: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = configured_cache(&listener, "", 32);
    let mut response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(body);
    let operation = async {
        let (value, request) = tokio::join!(
            cache.async_get_cache("response:key"),
            serve(&listener, &response)
        );
        assert!(
            request.starts_with("GET /responses/response/key?x-id=GetObject "),
            "{request}"
        );
        assert_eq!(value.is_some(), hit);
    };
    timeout(Duration::from_secs(5), operation).await.unwrap();
}

#[rstest]
#[case::missing("404 Not Found", "NoSuchKey")]
#[case::denied("403 Forbidden", "AccessDenied")]
#[case::service("500 Internal Server Error", "InternalError")]
#[tokio::test]
async fn storage_errors_are_misses(#[case] status: &str, #[case] code: &str) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = configured_cache(&listener, "", 4096);
    let body = format!("<Error><Code>{code}</Code></Error>");
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let operation = async {
        let (value, _) = tokio::join!(
            cache.async_get_cache("key"),
            serve(&listener, response.as_bytes())
        );
        assert!(value.is_none());
    };
    timeout(Duration::from_secs(5), operation).await.unwrap();
}

#[rstest]
#[case::object(r#"{"value":null}"#, r#"{"value":null}"#)]
#[case::pairs(r#"[["value",true]]"#, r#"{"value":true}"#)]
#[case::strings(r#"["ab", "cd"]"#, r#"{"a":"b","c":"d"}"#)]
#[case::duplicates(r#"[["a",1],["a",2]]"#, r#"{"a":2}"#)]
#[case::empty("[]", "{}")]
fn dictionary_conversion(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(
        decode_body(input.as_bytes()).unwrap(),
        decode_body(expected.as_bytes()).unwrap()
    );
}

#[rstest]
#[case::none("{'key': None}", r#"{"key":null}"#)]
#[case::boolean("{'key': True, 'other': False}", r#"{"key":true,"other":false}"#)]
#[case::tuple("(('key', 1),)", r#"{"key":1}"#)]
#[case::quotes(r#"{'key': "isn't JSON"}"#, r#"{"key":"isn't JSON"}"#)]
fn python_literals_use_ast_fallback(#[case] input: &str, #[case] expected: &str) {
    assert_eq!(
        decode_body(input.as_bytes()).unwrap(),
        decode_body(expected.as_bytes()).unwrap()
    );
}

#[rstest]
#[case::numeric_key("[[1,2]]")]
#[case::scalar("1")]
#[case::wrong_pair("[[\"a\"]]")]
#[case::wrong_string("[\"abc\"]")]
fn unsupported_body_is_rejected(#[case] input: &str) {
    decode_body(input.as_bytes()).unwrap_err();
}

#[tokio::test]
async fn pipeline_awaits_every_write() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = configured_cache(&listener, "", 4096);
    let entries: Vec<(String, CachedEntry)> = (0_u32..20)
        .map(|index| (format!("key{index}"), decode_body(b"{}").unwrap()))
        .collect();
    let operation = async {
        let ((), requests) = tokio::join!(
            cache.async_set_cache_pipeline(entries, None),
            serve_pipeline(&listener)
        );
        assert_eq!(requests.len(), 20);
        assert!((0_u32..20).all(|index| requests.iter().any(|request| {
            request.starts_with(&format!("PUT /responses/key{index}?x-id=PutObject "))
        })));
        cache.disconnect().await;
    };
    timeout(Duration::from_secs(5), operation).await.unwrap();
}

#[tokio::test]
async fn overflow_and_oversize_skip_writes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = configured_cache(&listener, "", 2);
    cache
        .async_set_cache("key", &decode_body(b"{}").unwrap(), Some(u64::MAX))
        .await;
    cache
        .async_set_cache("key", &decode_body(br#"{"large":true}"#).unwrap(), None)
        .await;
    timeout(Duration::from_millis(20), listener.accept())
        .await
        .unwrap_err();
}

#[tokio::test]
async fn stalled_headers_time_out() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = configured_cache(&listener, "", 4096);
    let operation = async {
        let (connection, value) = tokio::join!(listener.accept(), cache.async_get_cache("key"));
        drop(connection.unwrap());
        assert!(value.is_none());
    };
    timeout(Duration::from_secs(3), operation).await.unwrap();
}

#[tokio::test]
async fn failed_put_is_swallowed() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = configured_cache(&listener, "", 4096);
    let value = decode_body(b"{}").unwrap();
    let operation = async {
        let ((), request) = tokio::join!(cache.async_set_cache("key", &value, None), serve(&listener,
            b"HTTP/1.1 403 Forbidden\r\nContent-Length: 40\r\nConnection: close\r\n\r\n<Error><Code>AccessDenied</Code></Error>"));
        assert!(request.starts_with("PUT "));
    };
    timeout(Duration::from_secs(5), operation).await.unwrap();
}

#[tokio::test]
async fn chunked_body_is_bounded() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cache = configured_cache(&listener, "", 2);
    let operation = async {
        let (value, _) = tokio::join!(cache.async_get_cache("key"), serve(&listener,
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\nnull\r\n0\r\n\r\n"));
        assert!(value.is_none());
    };
    timeout(Duration::from_secs(5), operation).await.unwrap();
}

async fn serve_pipeline(listener: &TcpListener) -> Vec<String> {
    let mut requests = Vec::new();
    for _ in 0_u32..20 {
        requests.push(
            serve(
                listener,
                b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await,
        );
    }
    requests
}

#[tokio::test]
async fn operation_cap_bounds_stalled_calls() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let yaml = format!(
        "litellm_settings:\n  cache_params:\n    s3_bucket_name: responses\n    s3_endpoint_url: http://{address}\n    s3_aws_access_key_id: test-key\n    s3_aws_secret_access_key: test-secret\n    s3_config:\n      retries:\n        max_attempts: 0\n      s3:\n        addressing_style: path\n"
    );
    let cache = cache_from(&yaml, 4096, Duration::from_millis(500));
    let value = decode_body(b"{}").unwrap();
    let operation = async {
        let (connection, cached) = tokio::join!(listener.accept(), cache.async_get_cache("key"));
        assert!(cached.is_none());
        let (upload, ()) = tokio::join!(
            listener.accept(),
            cache.async_set_cache("key", &value, None)
        );
        drop((connection.unwrap(), upload.unwrap()));
    };
    timeout(Duration::from_secs(3), operation).await.unwrap();
}

#[tokio::test]
async fn absent_credentials_do_not_use_default_chain() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut settings = ConfigLoader {
        lookup: BTreeMap::new(),
    }
    .load("litellm_settings:\n  cache_params:\n    s3_bucket_name: responses\n")
    .unwrap()
    .sections
    .s3_response_cache
    .unwrap();
    settings.endpoint_url = Some(format!("http://{}", listener.local_addr().unwrap()));
    let transport = aws_smithy_http_client::Builder::new()
        .tls_provider(Provider::Rustls(CryptoMode::AwsLc))
        .build_https();
    let cache = S3Cache::new(
        &settings,
        SharedHttpClient::new(transport),
        SystemTime::now,
        NonZeroUsize::new(1024).unwrap(),
        Duration::from_secs(2),
    );
    let operation = async {
        let (value, request) = tokio::join!(
            cache.async_get_cache("key"),
            serve(
                &listener,
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}"
            )
        );
        assert!(value.is_some());
        assert!(!request.to_lowercase().contains("authorization:"));
    };
    timeout(Duration::from_secs(2), operation).await.unwrap();
}
