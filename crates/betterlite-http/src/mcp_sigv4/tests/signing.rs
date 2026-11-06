#![cfg(test)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use aws_credential_types::Credentials;
use aws_credential_types::provider::error::CredentialsError;
use aws_credential_types::provider::{ProvideCredentials, SharedCredentialsProvider, future};
use aws_smithy_async::time::SharedTimeSource;
use bytes::Bytes;
use http::Request;

use super::fixture::{Clock, Reply, VECTOR_TIME, built, explicit_settings, mcp_request};
use crate::mcp_sigv4::McpSigV4Auth;
use crate::mcp_sigv4::McpSigV4Settings;
use crate::mcp_sigv4::cache::CredentialCache;
use crate::mcp_sigv4::error::McpSigV4Error;

fn vector_settings() -> McpSigV4Settings {
    McpSigV4Settings {
        aws_service_name: Some("service".to_owned()),
        ..explicit_settings()
    }
}

fn authorization(request: &Request<Bytes>) -> String {
    request.headers()["authorization"]
        .to_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn sign_matches_the_aws_get_vanilla_vector() {
    let (auth, _recorder, _clock) = built(vector_settings(), &[], Reply::Refused).await;
    let request = Request::get("https://example.amazonaws.com/")
        .body(Bytes::new())
        .unwrap();
    let signed = auth.auth_flow(request).await.unwrap();
    assert_eq!(
        authorization(&signed),
        "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"
    );
    assert_eq!(signed.headers()["x-amz-date"], "20150830T123600Z");
}

#[tokio::test]
async fn sign_covers_every_header_except_the_ones_botocore_skips() {
    let (auth, _recorder, _clock) = built(explicit_settings(), &[], Reply::Refused).await;
    let request = Request::post("https://mcp.example.com/mcp")
        .header("content-type", "application/json")
        .header("x-custom", "1")
        .header("user-agent", "httpx")
        .header("connection", "keep-alive")
        .body(Bytes::new())
        .unwrap();
    let signed = auth.auth_flow(request).await.unwrap();
    assert!(
        authorization(&signed).contains("SignedHeaders=content-type;host;x-amz-date;x-custom,")
    );
    assert_eq!(signed.headers()["user-agent"], "httpx");
}

#[tokio::test]
async fn sign_does_not_add_host_or_content_length_headers() {
    let (auth, _recorder, _clock) = built(explicit_settings(), &[], Reply::Refused).await;
    let signed = auth.auth_flow(mcp_request("{\"a\":1}")).await.unwrap();
    assert!(!signed.headers().contains_key("host"));
    assert!(!signed.headers().contains_key("content-length"));
    assert!(signed.headers().get("authorization").is_some());
    assert!(signed.headers().get("x-amz-date").is_some());
}

#[tokio::test]
async fn sign_covers_the_request_body() {
    let (auth, _recorder, _clock) = built(explicit_settings(), &[], Reply::Refused).await;
    let first = auth.auth_flow(mcp_request("{\"a\":1}")).await.unwrap();
    let second = auth.auth_flow(mcp_request("{\"a\":2}")).await.unwrap();
    assert_ne!(authorization(&first), authorization(&second));
    assert_eq!(first.body().as_ref(), b"{\"a\":1}");
}

#[tokio::test]
async fn sign_replaces_authorization_and_date_headers_from_the_caller() {
    let (auth, _recorder, _clock) = built(explicit_settings(), &[], Reply::Refused).await;
    let request = Request::post("https://mcp.example.com/mcp")
        .header("authorization", "Bearer stale")
        .header("x-amz-date", "20000101T000000Z")
        .body(Bytes::new())
        .unwrap();
    let signed = auth.auth_flow(request).await.unwrap();
    assert!(authorization(&signed).starts_with("AWS4-HMAC-SHA256 "));
    assert_eq!(signed.headers().get_all("authorization").iter().count(), 1);
    assert_eq!(signed.headers()["x-amz-date"], "20150830T123600Z");
}

#[tokio::test]
async fn sign_joins_repeated_headers_with_a_comma_like_httpx() {
    let (auth, _recorder, _clock) = built(explicit_settings(), &[], Reply::Refused).await;
    let repeated = Request::post("https://mcp.example.com/mcp")
        .header("x-tag", "a")
        .header("x-tag", "b")
        .body(Bytes::new())
        .unwrap();
    let joined = Request::post("https://mcp.example.com/mcp")
        .header("x-tag", "a, b")
        .body(Bytes::new())
        .unwrap();
    let first = auth.auth_flow(repeated).await.unwrap();
    let second = auth.auth_flow(joined).await.unwrap();
    assert_eq!(authorization(&first), authorization(&second));
}

#[tokio::test]
async fn sign_uses_a_precomputed_content_hash_when_the_caller_sent_one() {
    let (auth, _recorder, _clock) = built(explicit_settings(), &[], Reply::Refused).await;
    let hashed = |body: &'static str| {
        Request::post("https://mcp.example.com/mcp")
            .header("x-amz-content-sha256", "UNSIGNED-PAYLOAD")
            .body(Bytes::from_static(body.as_bytes()))
            .unwrap()
    };
    let first = auth.auth_flow(hashed("one")).await.unwrap();
    let second = auth.auth_flow(hashed("two")).await.unwrap();
    assert_eq!(authorization(&first), authorization(&second));
}

#[tokio::test]
async fn sign_each_request_gets_a_fresh_signature() {
    let (auth, _recorder, clock) = built(explicit_settings(), &[], Reply::Refused).await;
    let first = auth.auth_flow(mcp_request("{}")).await.unwrap();
    clock.set(VECTOR_TIME + 60);
    let second = auth.auth_flow(mcp_request("{}")).await.unwrap();
    assert_eq!(second.headers()["x-amz-date"], "20150830T123700Z");
    assert_ne!(authorization(&first), authorization(&second));
}

#[tokio::test]
async fn sign_rejects_a_date_header_instead_of_dropping_it() {
    let (auth, _recorder, _clock) = built(explicit_settings(), &[], Reply::Refused).await;
    let request = Request::post("https://mcp.example.com/mcp")
        .header("date", "Sun, 30 Aug 2015 12:36:00 GMT")
        .body(Bytes::new())
        .unwrap();
    let error = auth.auth_flow(request).await.unwrap_err();
    assert!(matches!(error, McpSigV4Error::DateHeader));
}

#[tokio::test]
async fn sign_rejects_a_header_value_that_is_not_visible_ascii() {
    let (auth, _recorder, _clock) = built(explicit_settings(), &[], Reply::Refused).await;
    let request = Request::post("https://mcp.example.com/mcp")
        .header("x-bytes", http::HeaderValue::from_bytes(&[0xE9]).unwrap())
        .body(Bytes::new())
        .unwrap();
    let error = auth.auth_flow(request).await.unwrap_err();
    assert!(matches!(error, McpSigV4Error::HeaderValue { name, .. } if name == "x-bytes"));
}

#[derive(Debug)]
struct Rotating {
    calls: Arc<AtomicUsize>,
}

impl ProvideCredentials for Rotating {
    fn provide_credentials<'life>(&'life self) -> future::ProvideCredentials<'life>
    where
        Self: 'life,
    {
        let number = self.calls.fetch_add(1, Ordering::SeqCst);
        let expiry = UNIX_EPOCH
            .checked_add(Duration::from_secs(VECTOR_TIME.saturating_add(3600)))
            .unwrap();
        let credentials = Credentials::new(
            format!("AKIDROTATE{number}"),
            "secret",
            None,
            Some(expiry),
            "rotating",
        );
        future::ProvideCredentials::ready(Ok::<_, CredentialsError>(credentials))
    }
}

#[tokio::test]
async fn sign_refreshes_credentials_close_to_their_expiry() {
    let clock = Clock::at(VECTOR_TIME);
    let (sources, _recorder) = super::fixture::sources(&[], &[], &clock, Reply::Refused);
    let calls = Arc::new(AtomicUsize::new(0));
    let mut auth = McpSigV4Auth::new(explicit_settings(), sources)
        .await
        .unwrap();
    let rotating = Rotating {
        calls: Arc::clone(&calls),
    };
    auth.cache = CredentialCache::new(
        SharedCredentialsProvider::new(rotating),
        SharedTimeSource::new(clock.clone()),
    );
    let first = auth.auth_flow(mcp_request("{}")).await.unwrap();
    let again = auth.auth_flow(mcp_request("{}")).await.unwrap();
    assert!(authorization(&first).contains("Credential=AKIDROTATE0/"));
    assert!(authorization(&again).contains("Credential=AKIDROTATE0/"));
    clock.set(VECTOR_TIME + 3600 - 60);
    let refreshed = auth.auth_flow(mcp_request("{}")).await.unwrap();
    assert!(authorization(&refreshed).contains("Credential=AKIDROTATE1/"));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn sign_reads_a_plus_in_the_query_as_a_space() {
    let (auth, _recorder, _clock) = built(vector_settings(), &[], Reply::Refused).await;
    let with_plus = Request::get("https://example.amazonaws.com/?Param1=value+1")
        .body(Bytes::new())
        .unwrap();
    let with_space = Request::get("https://example.amazonaws.com/?Param1=value%201")
        .body(Bytes::new())
        .unwrap();
    let plus = auth.auth_flow(with_plus).await.unwrap();
    let space = auth.auth_flow(with_space).await.unwrap();
    assert_eq!(authorization(&plus), authorization(&space));
}
