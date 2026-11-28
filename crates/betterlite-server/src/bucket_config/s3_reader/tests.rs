#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::s3_transport::download::EMPTY_SHA256;
use betterlite_logging::s3_transport::signing::{Clock, Credentials, SystemClock};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use super::addressing::object_url;
use super::{
    DEFAULT_REGION, ProxiedS3Get, REGION_VARIABLE, S3Get as _, S3GetRequest, S3Reader,
    S3TransportFailure, credential_settings, region_from_environment,
};
use crate::bucket_config::object::BucketObjectReader as _;
use crate::bucket_config::test_support::{
    ACCESS_KEY, FakeS3, SECRET_KEY, credential_environment, environment_without_keys,
};

const FIXED_TIME: u64 = 1_700_000_000;

fn fixed_clock() -> Arc<dyn Clock> {
    Arc::new(FixedClock)
}

#[derive(Debug, Clone, Copy)]
struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> SystemTime {
        UNIX_EPOCH
            .checked_add(Duration::from_secs(FIXED_TIME))
            .unwrap()
    }
}

fn credentials() -> Credentials {
    Credentials {
        access_key_id: SecretString::from(ACCESS_KEY.to_owned()),
        secret_access_key: SecretString::from(SECRET_KEY.to_owned()),
        session_token: None,
    }
}

fn signed_header<'headers>(
    headers: &'headers [(String, String)],
    name: &str,
) -> Option<&'headers str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn reader(transport: Arc<FakeS3>, bucket_name: &str) -> S3Reader {
    S3Reader {
        bucket_name: bucket_name.to_owned(),
        region: "us-east-1".to_owned(),
        credentials: Some(credentials()),
        transport,
        clock: fixed_clock(),
    }
}

#[test]
fn a_virtual_hosted_url_carries_the_bucket_in_the_host() {
    assert_eq!(
        object_url("config-bucket", "us-east-1", "models/config.yaml"),
        "https://config-bucket.s3.us-east-1.amazonaws.com/models/config.yaml"
    );
}

#[test]
fn a_dotted_bucket_name_is_read_path_style() {
    assert_eq!(
        object_url("config.bucket", "us-east-1", "models/config.yaml"),
        "https://s3.us-east-1.amazonaws.com/config.bucket/models/config.yaml"
    );
}

#[rstest]
#[case::uppercase("ConfigBucket")]
#[case::underscore("config_bucket")]
#[case::too_short("ab")]
#[case::leading_hyphen("-config")]
#[case::trailing_hyphen("config-")]
#[case::too_long("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")]
fn a_bucket_name_boto3_would_not_virtual_host_is_read_path_style(#[case] bucket_name: &str) {
    assert_eq!(
        object_url(bucket_name, "us-east-1", "config.yaml"),
        format!("https://s3.us-east-1.amazonaws.com/{bucket_name}/config.yaml")
    );
}

#[rstest]
#[case::shortest_dns_name("abc")]
#[case::longest_dns_name("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")]
fn a_bucket_name_boto3_would_virtual_host_uses_the_bucket_host(#[case] bucket_name: &str) {
    assert_eq!(
        object_url(bucket_name, "us-east-1", "config.yaml"),
        format!("https://{bucket_name}.s3.us-east-1.amazonaws.com/config.yaml")
    );
}

#[test]
fn a_china_region_virtual_hosted_url_uses_the_partition_dns_suffix() {
    assert_eq!(
        object_url("config-bucket", "cn-north-1", "models/config.yaml"),
        "https://config-bucket.s3.cn-north-1.amazonaws.com.cn/models/config.yaml"
    );
}

#[test]
fn a_china_region_path_style_url_uses_the_partition_dns_suffix() {
    assert_eq!(
        object_url("config.bucket", "cn-north-1", "models/config.yaml"),
        "https://s3.cn-north-1.amazonaws.com.cn/config.bucket/models/config.yaml"
    );
}

#[rstest]
#[case::plain("models/config.yaml", "models/config.yaml")]
#[case::space("team dir/config.yaml", "team%20dir/config.yaml")]
#[case::plus("a+b.yaml", "a%2Bb.yaml")]
#[case::non_ascii("\u{43a}\u{43b}\u{44e}\u{447}.yaml", "%D0%BA%D0%BB%D1%8E%D1%87.yaml")]
#[case::unreserved("a~b_c-d.e", "a~b_c-d.e")]
#[case::percent("%.yaml", "%25.yaml")]
#[case::double_slash("a//b.yaml", "a//b.yaml")]
fn object_url_encodes_each_key_segment(#[case] object_key: &str, #[case] expected: &str) {
    assert_eq!(
        object_url("bucket", "us-east-1", object_key),
        format!("https://bucket.s3.us-east-1.amazonaws.com/{expected}")
    );
}

#[test]
fn a_missing_region_variable_falls_back_to_the_default() {
    assert_eq!(region_from_environment(&BTreeMap::new()), DEFAULT_REGION);
}

#[test]
fn an_empty_region_variable_falls_back_to_the_default() {
    let environment = BTreeMap::from([(REGION_VARIABLE.to_owned(), String::new())]);
    assert_eq!(region_from_environment(&environment), DEFAULT_REGION);
}

#[test]
fn the_region_variable_names_the_signing_region() {
    let environment = BTreeMap::from([(REGION_VARIABLE.to_owned(), "eu-west-1".to_owned())]);
    assert_eq!(region_from_environment(&environment), "eu-west-1");
}

#[test]
fn the_system_clock_reads_the_wall_clock() {
    assert!(SystemClock.now() > UNIX_EPOCH);
}

#[test]
fn the_credential_settings_carry_only_the_region() {
    let settings = credential_settings("eu-west-1");

    assert_eq!(settings.region_name.as_deref(), Some("eu-west-1"));
    assert!(settings.access_key_id.is_none());
    assert!(settings.secret_access_key.is_none());
    assert!(settings.session_token.is_none());
    assert!(settings.profile_name.is_none());
    assert!(settings.role_arn.is_none());
    assert!(settings.web_identity_token_file.is_none());
}

#[tokio::test]
async fn initialize_resolves_the_environment_credentials() {
    let reader = S3Reader::initialize(
        "bucket",
        &credential_environment(),
        Arc::new(FakeS3::default()),
    )
    .await;

    let resolved = reader.credentials.as_ref().unwrap();
    assert_eq!(resolved.access_key_id.expose_secret(), ACCESS_KEY);
    assert_eq!(resolved.secret_access_key.expose_secret(), SECRET_KEY);
    assert!(resolved.session_token.is_none());
    assert_eq!(reader.region, DEFAULT_REGION);
}

#[tokio::test]
async fn initialize_keeps_the_session_token() {
    let mut environment = credential_environment();
    environment.insert("AWS_SESSION_TOKEN".to_owned(), "token".to_owned());

    let reader = S3Reader::initialize("bucket", &environment, Arc::new(FakeS3::default())).await;

    let resolved = reader.credentials.as_ref().unwrap();
    assert_eq!(
        resolved
            .session_token
            .as_ref()
            .map(|token| token.expose_secret().to_owned()),
        Some("token".to_owned())
    );
}

#[tokio::test]
async fn initialize_reads_the_region_from_the_environment() {
    let mut environment = credential_environment();
    environment.insert(REGION_VARIABLE.to_owned(), "eu-west-1".to_owned());
    let transport = FakeS3::answering(200, b"ok");
    let transport_handle = Arc::clone(&transport);

    let reader = S3Reader::initialize("bucket", &environment, transport_handle).await;

    assert_eq!(reader.region, "eu-west-1");
    assert_eq!(
        reader.read_object("config.yaml").await.as_deref(),
        Some(b"ok".as_slice())
    );
    assert_eq!(
        transport.urls(),
        vec!["https://bucket.s3.eu-west-1.amazonaws.com/config.yaml".to_owned()]
    );
}

#[tokio::test]
async fn a_reader_without_resolvable_credentials_reads_nothing() {
    let environment = environment_without_keys();
    let transport = Arc::new(FakeS3::default());
    let transport_handle = Arc::clone(&transport);

    let reader = S3Reader::initialize("bucket", &environment, transport_handle).await;

    assert!(reader.credentials.is_none());
    assert!(reader.read_object("config.yaml").await.is_none());
    assert!(transport.urls().is_empty());
}

#[tokio::test]
async fn a_signed_get_carries_the_sigv4_headers() {
    let reader = reader(Arc::new(FakeS3::default()), "bucket");
    let url = "https://bucket.s3.us-east-1.amazonaws.com/models/config.yaml";

    let headers = reader.signed_headers(url, &credentials()).unwrap();

    assert_eq!(
        headers.first(),
        Some(&("x-amz-content-sha256".to_owned(), EMPTY_SHA256.to_owned()))
    );
    assert_eq!(
        signed_header(&headers, "x-amz-date"),
        Some("20231114T221320Z")
    );
    let authorization = signed_header(&headers, "authorization");
    assert_eq!(
        authorization,
        Some(
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20231114/us-east-1/s3/aws4_request, \
             SignedHeaders=host;x-amz-content-sha256;x-amz-date, \
             Signature=f608f324b3dd0d74b2c11ab9141eb05e34a81b19520eb88cd6026ea781a11518"
        )
    );
}

#[tokio::test]
async fn a_signed_get_carries_the_session_token() {
    let reader = reader(Arc::new(FakeS3::default()), "bucket");
    let mut with_token = credentials();
    with_token.session_token = Some(SecretString::from("token".to_owned()));

    let headers = reader
        .signed_headers(
            "https://bucket.s3.us-east-1.amazonaws.com/config.yaml",
            &with_token,
        )
        .unwrap();

    assert_eq!(
        signed_header(&headers, "x-amz-security-token"),
        Some("token")
    );
}

#[tokio::test]
async fn a_get_with_a_key_needing_encoding_is_signed_as_sent() {
    let transport = FakeS3::answering(200, b"model_list: []");
    let reader = reader(Arc::clone(&transport), "bucket");

    let body = reader.read_object("team dir/config.yaml").await;

    assert_eq!(body.as_deref(), Some(b"model_list: []".as_slice()));
    assert_eq!(
        transport.urls(),
        vec!["https://bucket.s3.us-east-1.amazonaws.com/team%20dir/config.yaml".to_owned()]
    );
    let headers = transport.headers();
    assert_eq!(
        signed_header(&headers, "x-amz-date"),
        Some("20231114T221320Z")
    );
}

#[tokio::test]
async fn a_200_answer_is_the_object_body() {
    let transport = FakeS3::answering(200, b"model_list: []");
    let reader = reader(transport, "bucket");

    assert_eq!(
        reader.read_object("config.yaml").await.as_deref(),
        Some(b"model_list: []".as_slice())
    );
}

#[tokio::test]
async fn a_non_200_answer_reads_as_missing() {
    let transport = FakeS3::answering(404, b"no such key");
    let reader = reader(transport, "bucket");

    assert!(reader.read_object("config.yaml").await.is_none());
}

#[tokio::test]
async fn a_transport_error_reads_as_missing() {
    let reader = reader(
        FakeS3::failing(S3TransportFailure::Transport(TransportError::Closed)),
        "bucket",
    );

    assert!(reader.read_object("config.yaml").await.is_none());
}

#[tokio::test]
async fn a_bucket_name_that_cannot_be_signed_reads_as_missing() {
    let transport = Arc::new(FakeS3::default());
    let reader = reader(Arc::clone(&transport), "bad bucket");

    assert!(reader.read_object("config.yaml").await.is_none());
    assert!(transport.urls().is_empty());
}

#[tokio::test]
async fn a_reader_without_credentials_never_reaches_the_transport() {
    let transport = Arc::new(FakeS3::default());
    let mut reader = reader(Arc::clone(&transport), "bucket");
    reader.credentials = None;

    assert!(reader.read_object("config.yaml").await.is_none());
    assert!(transport.urls().is_empty());
}

#[tokio::test]
async fn a_dotted_bucket_name_is_read_path_style_over_the_transport() {
    let transport = FakeS3::answering(200, b"ok");
    let reader = reader(Arc::clone(&transport), "config.bucket");

    assert_eq!(
        reader.read_object("models/config.yaml").await.as_deref(),
        Some(b"ok".as_slice())
    );
    assert_eq!(
        transport.urls(),
        vec!["https://s3.us-east-1.amazonaws.com/config.bucket/models/config.yaml".to_owned()]
    );
}

async fn serving(status: u16, body: &'static str) -> (String, JoinSet<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let application = Router::new().route(
        "/{*path}",
        get(move || async move { (StatusCode::from_u16(status).unwrap(), body) }),
    );
    let mut server = JoinSet::new();
    server.spawn(async move {
        axum::serve(listener, application).await.unwrap();
    });
    (format!("http://{address}/bucket/config.yaml"), server)
}

fn proxied() -> ProxiedS3Get {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    ProxiedS3Get::new(&Arc::new(policy)).unwrap()
}

#[tokio::test]
async fn the_proxied_transport_returns_the_status_and_body() {
    let (url, _server) = serving(200, "model_list: []").await;

    let response = proxied()
        .get(S3GetRequest {
            url,
            headers: Vec::new(),
        })
        .await
        .unwrap();

    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"model_list: []");
}

#[tokio::test]
async fn the_proxied_transport_keeps_a_non_200_status() {
    let (url, _server) = serving(403, "forbidden").await;

    let response = proxied()
        .get(S3GetRequest {
            url,
            headers: Vec::new(),
        })
        .await
        .unwrap();

    assert_eq!(response.status, 403);
}

#[tokio::test]
async fn the_proxied_transport_refuses_a_url_that_is_not_valid() {
    let error = proxied()
        .get(S3GetRequest {
            url: "not a url".to_owned(),
            headers: Vec::new(),
        })
        .await
        .unwrap_err();

    assert!(matches!(error, S3TransportFailure::Url(_)));
}

#[tokio::test]
async fn the_proxied_transport_refuses_a_header_name_that_is_not_valid() {
    let (url, _server) = serving(200, "ok").await;
    let error = proxied()
        .get(S3GetRequest {
            url,
            headers: vec![("bad header".to_owned(), "value".to_owned())],
        })
        .await
        .unwrap_err();

    assert!(matches!(error, S3TransportFailure::HeaderName { .. }));
}

#[tokio::test]
async fn the_proxied_transport_refuses_a_header_value_that_is_not_valid() {
    let (url, _server) = serving(200, "ok").await;
    let error = proxied()
        .get(S3GetRequest {
            url,
            headers: vec![("x-test".to_owned(), "line\nbreak".to_owned())],
        })
        .await
        .unwrap_err();

    assert!(matches!(error, S3TransportFailure::HeaderValue { .. }));
}

#[tokio::test]
async fn the_proxied_transport_reports_a_connection_that_fails() {
    let error = proxied()
        .get(S3GetRequest {
            url: "http://127.0.0.1:1/bucket/config.yaml".to_owned(),
            headers: Vec::new(),
        })
        .await
        .unwrap_err();

    assert!(matches!(error, S3TransportFailure::Transport(_)));
}
