#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::http::{HeaderMap, Method};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_mcp::credential_contracts::auth_config::AuthConfig;
use betterlite_mcp::credential_contracts::auth_config::oauth::{
    DEFAULT_SUBJECT_TOKEN_TYPE, TokenExchangeConfig, TokenExchangeProfile,
};
use betterlite_mcp::credential_contracts::header_carrier::HeaderCarrier;
use betterlite_mcp::credential_contracts::server_spec::ServerSpec;
use rstest::rstest;
use secrecy::SecretString;
use url::Url;

use super::super::token_exchanger::{
    DEFAULT_TTL, EXPIRY_BUFFER, ExchangeHttpPost as _, ExchangeRejection, MIN_TTL,
    TOKEN_EXCHANGE_GRANT,
};
use super::{
    CACHE_MAX_SIZE, EXCHANGE_TIMEOUT, ExchangeTransport, TokenEndpointPost, build_token_exchanger,
};

#[derive(Clone)]
struct FakeTransport {
    state: Arc<Mutex<TransportState>>,
}

#[derive(Default)]
struct TransportState {
    responses: VecDeque<Result<UpstreamResponse, TransportError>>,
    requests: Vec<RecordedRequest>,
}

#[derive(Clone)]
struct RecordedRequest {
    method: Method,
    url: Url,
    headers: HeaderMap,
    body: Vec<u8>,
    deadline: Duration,
}

impl FakeTransport {
    fn new(responses: Vec<Result<UpstreamResponse, TransportError>>) -> Self {
        Self {
            state: Arc::new(Mutex::new(TransportState {
                responses: VecDeque::from(responses),
                requests: Vec::new(),
            })),
        }
    }

    fn answering(status: u16, raw: &str) -> Self {
        Self::new(vec![Ok(UpstreamResponse {
            status,
            headers: HeaderMap::new(),
            body: raw.as_bytes().to_vec(),
        })])
    }

    fn failing() -> Self {
        Self::new(vec![Err(TransportError::Closed)])
    }

    fn requests(&self) -> Vec<RecordedRequest> {
        self.state.lock().unwrap().requests.clone()
    }
}

#[async_trait]
impl ExchangeTransport for FakeTransport {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        let body = match request.body {
            RequestBody::Bytes { data, .. } => data,
            RequestBody::Empty | RequestBody::Json(_) | RequestBody::Multipart { .. } => Vec::new(),
        };
        let mut state = self.state.lock().unwrap();
        state.requests.push(RecordedRequest {
            method: request.method,
            url: request.url,
            headers: request.headers,
            body,
            deadline: request.deadline,
        });
        state
            .responses
            .pop_front()
            .unwrap_or(Err(TransportError::Closed))
    }
}

fn form() -> BTreeMap<String, SecretString> {
    BTreeMap::from([
        (
            "grant_type".to_owned(),
            SecretString::from(TOKEN_EXCHANGE_GRANT),
        ),
        ("subject_token".to_owned(), SecretString::from("a b")),
        ("client_secret".to_owned(), SecretString::from("csec")),
    ])
}

fn auth_headers() -> BTreeMap<String, SecretString> {
    BTreeMap::from([(
        "Authorization".to_owned(),
        SecretString::from("Basic Y2lkOmNzZWM="),
    )])
}

fn post(transport: FakeTransport) -> TokenEndpointPost<FakeTransport> {
    TokenEndpointPost {
        transport,
        timeout: EXCHANGE_TIMEOUT,
    }
}

fn config() -> TokenExchangeConfig {
    TokenExchangeConfig {
        header: HeaderCarrier::default(),
        profile: TokenExchangeProfile::Rfc8693,
        subject_token_type: DEFAULT_SUBJECT_TOKEN_TYPE.to_owned(),
        token_exchange_endpoint: Some("https://idp.example.com/token".to_owned()),
        audience: None,
        client_id: Some("cid".to_owned()),
        client_secret: Some(SecretString::from("csec")),
        token_endpoint_auth_method: None,
        scopes: Vec::new(),
    }
}

fn server() -> ServerSpec {
    ServerSpec {
        server_id: "srv".to_owned(),
        resource: "https://up.example.com".to_owned(),
        config: AuthConfig::TokenExchange(config()),
    }
}

#[tokio::test]
async fn the_post_sends_a_form_urlencoded_request_with_accept_and_client_auth() {
    let transport = FakeTransport::answering(200, r#"{"access_token":"x"}"#);
    let post = post(transport.clone());
    let body = post
        .post("https://idp.example.com/token", &form(), &auth_headers())
        .await
        .unwrap()
        .unwrap();
    assert!(body.contains_key("access_token"));
    let requests = transport.requests();
    assert_eq!(requests[0].method, Method::POST);
    assert_eq!(requests[0].url.as_str(), "https://idp.example.com/token");
    assert_eq!(
        requests[0].headers.get("accept").unwrap().to_str().unwrap(),
        "application/json"
    );
    assert_eq!(
        requests[0]
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap(),
        "Basic Y2lkOmNzZWM="
    );
    assert_eq!(requests[0].deadline, EXCHANGE_TIMEOUT);
    assert_eq!(
        String::from_utf8(requests[0].body.clone()).unwrap(),
        "client_secret=csec&grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Atoken-exchange&subject_token=a+b"
    );
}

#[rstest]
#[case::invalid_client("invalid_client")]
#[case::unauthorized_client("unauthorized_client")]
#[case::unsupported_grant_type("unsupported_grant_type")]
#[case::invalid_target("invalid_target")]
#[case::invalid_scope("invalid_scope")]
#[tokio::test]
async fn a_gateway_fault_code_is_a_client_fault(#[case] code: &str) {
    let raw = format!(r#"{{"error":"{code}"}}"#);
    let post = post(FakeTransport::answering(400, &raw));
    let error = post
        .post("https://idp/token", &form(), &BTreeMap::new())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        ExchangeRejection::ClientFault {
            code: code.to_owned()
        }
    );
}

#[tokio::test]
async fn another_4xx_is_a_subject_rejection_with_the_claims() {
    let post = post(FakeTransport::answering(
        400,
        r#"{"error":"invalid_grant","claims":"{\"access_token\":{\"acrs\":{}}}"}"#,
    ));
    let error = post
        .post("https://idp/token", &form(), &BTreeMap::new())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        ExchangeRejection::SubjectRejected {
            detail: "IdP rejected the subject token (HTTP 400)".to_owned(),
            claims: Some("{\"access_token\":{\"acrs\":{}}}".to_owned()),
        }
    );
}

#[tokio::test]
async fn an_empty_claims_value_is_absent() {
    let post = post(FakeTransport::answering(
        401,
        r#"{"error":"invalid_grant","claims":""}"#,
    ));
    let error = post
        .post("https://idp/token", &form(), &BTreeMap::new())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        ExchangeRejection::SubjectRejected {
            detail: "IdP rejected the subject token (HTTP 401)".to_owned(),
            claims: None,
        }
    );
}

#[tokio::test]
async fn a_non_string_claims_value_is_absent() {
    let post = post(FakeTransport::answering(
        401,
        r#"{"error":"invalid_grant","claims":7}"#,
    ));
    let error = post
        .post("https://idp/token", &form(), &BTreeMap::new())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        ExchangeRejection::SubjectRejected {
            detail: "IdP rejected the subject token (HTTP 401)".to_owned(),
            claims: None,
        }
    );
}

#[tokio::test]
async fn a_4xx_without_an_error_code_is_a_subject_rejection() {
    let post = post(FakeTransport::answering(403, r#"{"detail":"nope"}"#));
    let error = post
        .post("https://idp/token", &form(), &BTreeMap::new())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        ExchangeRejection::SubjectRejected {
            detail: "IdP rejected the subject token (HTTP 403)".to_owned(),
            claims: None,
        }
    );
}

#[tokio::test]
async fn a_4xx_with_a_non_json_body_is_a_subject_rejection() {
    let post = post(FakeTransport::answering(400, "not json"));
    let error = post
        .post("https://idp/token", &form(), &BTreeMap::new())
        .await
        .unwrap_err();
    assert_eq!(
        error,
        ExchangeRejection::SubjectRejected {
            detail: "IdP rejected the subject token (HTTP 400)".to_owned(),
            claims: None,
        }
    );
}

#[tokio::test]
async fn a_4xx_with_a_non_string_error_code_is_a_subject_rejection() {
    let post = post(FakeTransport::answering(400, r#"{"error":7}"#));
    let error = post
        .post("https://idp/token", &form(), &BTreeMap::new())
        .await
        .unwrap_err();
    assert!(matches!(error, ExchangeRejection::SubjectRejected { .. }));
}

#[rstest]
#[case::server_error(500)]
#[case::service_unavailable(503)]
#[case::redirect(302)]
#[tokio::test]
async fn a_non_2xx_non_4xx_status_is_a_miss(#[case] status: u16) {
    let post = post(FakeTransport::answering(
        status,
        r#"{"error":"server_error"}"#,
    ));
    assert_eq!(
        post.post("https://idp/token", &form(), &BTreeMap::new())
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn a_transport_failure_is_a_miss() {
    let post = post(FakeTransport::failing());
    assert_eq!(
        post.post("https://idp/token", &form(), &BTreeMap::new())
            .await
            .unwrap(),
        None
    );
}

#[rstest]
#[case::array("[]")]
#[case::text(r#""token""#)]
#[case::number("7")]
#[case::invalid("{")]
#[case::empty("")]
#[tokio::test]
async fn a_2xx_body_that_is_not_a_json_object_is_a_miss(#[case] raw: &str) {
    let post = post(FakeTransport::answering(200, raw));
    assert_eq!(
        post.post("https://idp/token", &form(), &BTreeMap::new())
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn an_invalid_endpoint_url_is_a_miss() {
    let transport = FakeTransport::answering(200, r#"{"access_token":"x"}"#);
    let post = post(transport.clone());
    assert_eq!(
        post.post("not a url", &form(), &BTreeMap::new())
            .await
            .unwrap(),
        None
    );
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn the_composition_root_sizes_the_cache_and_uses_the_constants() {
    let exchanger = build_token_exchanger(FakeTransport::answering(
        200,
        r#"{"access_token":"x","expires_in":60}"#,
    ));
    assert_eq!(CACHE_MAX_SIZE, 500);
    assert_eq!(exchanger.cache.max_size, CACHE_MAX_SIZE);
    assert_eq!(exchanger.default_ttl, DEFAULT_TTL);
    assert_eq!(exchanger.min_ttl, MIN_TTL);
    assert_eq!(exchanger.expiry_buffer, EXPIRY_BUFFER);
    assert_eq!(exchanger.post.timeout, EXCHANGE_TIMEOUT);
}

#[tokio::test]
async fn each_build_gets_its_own_empty_cache() {
    let transport = FakeTransport::answering(200, r#"{"access_token":"x","expires_in":60}"#);
    let first = build_token_exchanger(transport.clone());
    first
        .exchange("jwt", &server(), &config(), "")
        .await
        .unwrap();
    assert_eq!(transport.requests().len(), 1);
    assert_eq!(first.cache.entries.lock().unwrap().len(), 1);
    let second = build_token_exchanger(transport.clone());
    assert!(second.cache.entries.lock().unwrap().is_empty());
}
