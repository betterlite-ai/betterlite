#![cfg(test)]

pub(crate) mod support;

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::StreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use http::{HeaderMap, Method};
use tokio::io::AsyncWriteExt as _;
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;
use url::Url;

use crate::json::{Json, Object};
use crate::ocr_auth::{AwsRequestSigner, AwsSigningContext, AwsSigningRequest};
use crate::ocr_config::decode::{decode_response, response_format};
use crate::ocr_config::{DecodedOcrResponse, OcrEnvironment};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::{OcrError, OcrTransportError};
use crate::ocr_http::{
    CallHooks, OcrCallContext, OcrWireRequest, body_document, build_http_request,
    guardrail_document, ocr, read_response_bytes, transform_request_body,
};
use crate::ocr_settings::OcrSettings;
use crate::request_format::RequestFormat;

use support::{
    MockServer, TestBody, TestConfig, TestPayload, chunked, delayed, http_client, object,
    ocr_client, prepared_request, reply,
};

struct TestHooks;

#[async_trait]
impl CallHooks for TestHooks {
    async fn before_provider_request(
        &self,
        mut request: OcrWireRequest,
    ) -> Result<OcrWireRequest, OcrError> {
        let Json::Object(body) = &mut request.body else {
            panic!("the hook sees the composed object body");
        };
        assert!(body.contains_key("document") || body.contains_key("type"));
        let rewrite = object(&[
            ("type", Json::Text("image_url".to_owned())),
            (
                "image_url",
                Json::Text("https://example.com/guardrail.png".to_owned()),
            ),
        ]);
        if body.contains_key("document") {
            let _previous = body.insert("document".to_owned(), Json::Object(rewrite));
        } else {
            *body = rewrite;
        }
        request
            .headers
            .push(("x-hook".to_owned(), "seen".to_owned()));
        Ok(request)
    }

    async fn response_received(&self, body: &[u8]) -> Result<(), OcrError> {
        assert!(body.windows(5).any(|window| window == b"hello"));
        Ok(())
    }
}

#[tokio::test]
async fn sends_one_request_in_order_and_runs_the_hooks_before_decoding() {
    let body = br#"{"pages":[{"index":0,"markdown":"hello"}],"model":"served-model"}"#;
    let server = MockServer::start(vec![reply("200 OK", &[], body)]).await;
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let params = object(&[
        ("req_format", Json::Text("litellm".to_owned())),
        ("future_option", Json::Bool(true)),
    ]);
    let request = prepared_request(format!("{}/base", server.base), params);
    let response = ocr(&TestConfig, &client, &request, &TestHooks)
        .await
        .expect("the call succeeds");
    assert_eq!(response.model, "requested-model");
    assert_eq!(response.pages.len(), 1);
    assert_eq!(response.pages[0].markdown, "hello");
    assert_eq!(response.provider_native_response, None);
    let requests = server.requests().await;
    assert_eq!(requests.len(), 1);
    let sent = &requests[0];
    assert!(sent.starts_with("POST /base/v1/ocr HTTP/1.1"), "{sent}");
    let lowered = sent.to_ascii_lowercase();
    assert!(lowered.contains("authorization: bearer test-key"), "{sent}");
    assert!(lowered.contains("x-hook: seen"), "{sent}");
    assert!(sent.contains("\"future_option\":true"), "{sent}");
    assert!(!sent.contains("req_format"), "{sent}");
}

#[tokio::test]
async fn reads_at_most_max_response_bytes() {
    let declared = MockServer::start(vec![reply("200 OK", &[], b"0123456789")]).await;
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let mut request = prepared_request(declared.base.clone(), Object::new());
    request.connection.max_response_bytes = 6;
    let error = ocr(&TestConfig, &client, &request, &NoHooks)
        .await
        .expect_err("a declared body over the cap fails");
    assert!(matches!(error, OcrError::TooLarge { limit: 6 }));

    let streamed = MockServer::start(vec![chunked(&[b"01234", b"56789"])]).await;
    let mut streamed_request = prepared_request(streamed.base.clone(), Object::new());
    streamed_request.connection.max_response_bytes = 6;
    let streamed_error = ocr(&TestConfig, &client, &streamed_request, &NoHooks)
        .await
        .expect_err("a streamed body over the cap fails");
    assert!(matches!(streamed_error, OcrError::TooLarge { limit: 6 }));
}

struct NoHooks;

#[async_trait]
impl CallHooks for NoHooks {
    async fn before_provider_request(
        &self,
        request: OcrWireRequest,
    ) -> Result<OcrWireRequest, OcrError> {
        Ok(request)
    }

    async fn response_received(&self, _body: &[u8]) -> Result<(), OcrError> {
        Ok(())
    }
}

#[tokio::test]
async fn non_2xx_becomes_the_provider_error_with_the_response_headers() {
    let server = MockServer::start(vec![reply(
        "500 Internal Server Error",
        &[("retry-after", "1")],
        b"boom",
    )])
    .await;
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let request = prepared_request(format!("{}/base", server.base), Object::new());
    let error = ocr(&TestConfig, &client, &request, &NoHooks)
        .await
        .expect_err("a provider failure is an error");
    let OcrError::Provider {
        status,
        body,
        headers,
    } = error
    else {
        panic!("the default error class is the provider error");
    };
    assert_eq!(status, 500);
    assert_eq!(body, "boom");
    assert!(
        headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("retry-after") && value == "1"),
        "{headers:?}"
    );
}

#[tokio::test]
async fn native_format_keeps_the_provider_json_in_the_native_slot() {
    let body =
        br#"{"pages":[{"index":0,"markdown":"hello"}],"model":"served-model","extra":"kept"}"#;
    let server =
        MockServer::start(vec![reply("200 OK", &[], body), reply("200 OK", &[], body)]).await;
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let params = object(&[("req_format", Json::Text("native".to_owned()))]);
    let request = prepared_request(format!("{}/base", server.base), params);
    let response = ocr(&TestConfig, &client, &request, &NoHooks)
        .await
        .expect("the native call succeeds");
    let expected: Json = serde_json::from_slice(body).expect("the mock body is JSON");
    assert_eq!(response.provider_native_response, Some(expected));
    assert_eq!(response.model, "requested-model");

    let litellm_request = prepared_request(format!("{}/base", server.base), Object::new());
    let litellm_response = ocr(&TestConfig, &client, &litellm_request, &NoHooks)
        .await
        .expect("the litellm call succeeds");
    assert_eq!(litellm_response.provider_native_response, None);
    assert_eq!(server.requests().await.len(), 2);
}

async fn fetch_url(text: &str) -> StreamResponse {
    let client = http_client(Duration::from_secs(5));
    let url = Url::parse(text).expect("the test URL parses");
    let request = OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers: HeaderMap::new(),
        body: RequestBody::Json(Vec::new()),
        deadline: Duration::from_secs(5),
    };
    client.send(request).await.expect("the test server answers")
}

#[tokio::test]
async fn a_declared_length_exactly_at_the_limit_is_accepted() {
    let server = MockServer::start(vec![reply("200 OK", &[], b"012345")]).await;
    let response = fetch_url(&format!("{}/document", server.base)).await;
    let bytes = read_response_bytes(response, 6)
        .await
        .expect("an exact declared length is accepted");
    assert_eq!(bytes, b"012345");
}

#[tokio::test]
async fn a_streamed_body_exactly_at_the_limit_is_accepted() {
    let server = MockServer::start(vec![chunked(&[b"012", b"345"])]).await;
    let response = fetch_url(&format!("{}/document", server.base)).await;
    let bytes = read_response_bytes(response, 6)
        .await
        .expect("an exact streamed body is accepted");
    assert_eq!(bytes, b"012345");
}

#[tokio::test]
async fn a_declared_length_over_the_limit_fails_before_the_body_arrives() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the lying server binds");
    let address = listener
        .local_addr()
        .expect("the lying server has an address");
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("the lying server accepts");
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nabc")
            .await
            .expect("the lying server writes");
        socket.shutdown().await.expect("the lying server closes");
    });
    let response = fetch_url(&format!("http://{address}/lying")).await;
    let error = read_response_bytes(response, 3)
        .await
        .expect_err("the declared length over the cap fails");
    assert!(matches!(error, OcrError::TooLarge { limit: 3 }));
    tasks
        .join_next()
        .await
        .expect("the lying server task runs")
        .expect("the lying server task does not panic");
}

#[tokio::test]
async fn a_failed_body_is_read_only_up_to_the_limit() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the failing server binds");
    let address = listener
        .local_addr()
        .expect("the failing server has an address");
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("the failing server accepts");
        socket
            .write_all(b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 10\r\n\r\n0123")
            .await
            .expect("the failing server writes");
        sleep(Duration::from_millis(100)).await;
        socket
            .write_all(b"45")
            .await
            .expect("the failing server writes");
        sleep(Duration::from_millis(300)).await;
        socket.shutdown().await.expect("the failing server closes");
    });
    let response = fetch_url(&format!("http://{address}/failed")).await;
    let error = read_response_bytes(response, 6)
        .await
        .expect_err("the failed answer is an error");
    let OcrError::Transport(OcrTransportError::Http { status, body }) = error else {
        panic!("the failed answer keeps its status");
    };
    assert_eq!(status, 500);
    assert_eq!(body, "012345");
    tasks
        .join_next()
        .await
        .expect("the failing server task runs")
        .expect("the failing server task does not panic");
}

#[tokio::test]
async fn read_response_bytes_decodes_success_and_reports_failed_statuses() {
    let server = MockServer::start(vec![reply(
        "200 OK",
        &[],
        br#"{"pages":[{"index":0,"markdown":"hello"}]}"#,
    )])
    .await;
    let response = fetch_url(&format!("{}/document", server.base)).await;
    let bytes = read_response_bytes(response, 1024)
        .await
        .expect("the success body reads");
    let decoded: DecodedOcrResponse<TestPayload> =
        decode_response(&bytes, true).expect("the success body decodes");
    assert_eq!(decoded.data.pages.len(), 1);
    assert_eq!(decoded.data.pages[0].markdown, "hello");
    assert!(decoded.native.is_some());
    assert!(decoded.text.contains("hello"));

    let failed_server = MockServer::start(vec![reply("404 Not Found", &[], b"missing")]).await;
    let failed_response = fetch_url(&format!("{}/document", failed_server.base)).await;
    let error = read_response_bytes(failed_response, 1024)
        .await
        .expect_err("a failed status is an error");
    let OcrError::Transport(OcrTransportError::Http { status, body }) = error else {
        panic!("the failed status is carried on the transport error");
    };
    assert_eq!(status, 404);
    assert_eq!(body, "missing");
}

#[test]
fn response_format_defaults_and_rejects_unknown_values() {
    assert_eq!(
        response_format(&Object::new()).expect("absent defaults"),
        RequestFormat::Litellm
    );
    let null = object(&[("req_format", Json::Null)]);
    assert_eq!(
        response_format(&null).expect("null defaults"),
        RequestFormat::Litellm
    );
    let native = object(&[("req_format", Json::Text("native".to_owned()))]);
    assert_eq!(
        response_format(&native).expect("native parses"),
        RequestFormat::Native
    );
    let invalid = object(&[("req_format", Json::Text("Native".to_owned()))]);
    assert!(matches!(
        response_format(&invalid),
        Err(OcrError::RequestFormat)
    ));
}

#[tokio::test]
async fn a_stalled_provider_times_out() {
    let server = MockServer::start(vec![delayed(
        reply("200 OK", &[], b"abc"),
        Duration::from_millis(300),
    )])
    .await;
    let client = ocr_client(OcrSettings::default(), Duration::from_millis(50));
    let request = prepared_request(format!("{}/base", server.base), Object::new());
    let error = ocr(&TestConfig, &client, &request, &NoHooks)
        .await
        .expect_err("the stalled server times out");
    assert_eq!(error.http_status_code(), Some(408));
}

fn provider_document() -> OcrDocument {
    OcrDocument::DocumentUrl {
        document_url: "data:application/pdf;base64,YWJj".to_owned(),
        extra: BTreeMap::new(),
    }
}

#[test]
fn build_http_request_carries_the_json_body_and_deadline() {
    let request = prepared_request("http://example.com".to_owned(), Object::new());
    let body = TestBody {
        model: "requested-model".to_owned(),
        document: provider_document(),
    };
    let outbound = build_http_request(&request, "http://example.com/v1/ocr", Vec::new(), &body)
        .expect("the request builds");
    assert_eq!(outbound.url.as_str(), "http://example.com/v1/ocr");
    assert_eq!(outbound.deadline, request.connection.timeout);
    assert_eq!(
        outbound
            .headers
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    let RequestBody::Json(bytes) = outbound.body else {
        panic!("the provider body is JSON");
    };
    let decoded: Json = serde_json::from_slice(&bytes).expect("the body is JSON");
    let Json::Object(sent) = decoded else {
        panic!("the body is an object");
    };
    assert_eq!(
        sent.get("model"),
        Some(&Json::Text("requested-model".to_owned()))
    );
}

#[tokio::test]
async fn guardrail_document_lets_the_hook_rewrite_the_document() {
    let request = prepared_request("http://example.com".to_owned(), Object::new());
    let (document, headers) =
        guardrail_document(&request, "http://example.com/v1/ocr", &[], &TestHooks)
            .await
            .expect("the document passes the hook");
    assert_eq!(document.source(), "https://example.com/guardrail.png");
    assert!(
        headers
            .iter()
            .any(|(name, value)| name == "x-hook" && value == "seen"),
        "{headers:?}"
    );
}

#[test]
fn body_document_reads_the_document_from_a_composed_body() {
    let body = Json::Object(object(&[
        ("model", Json::Text("requested-model".to_owned())),
        (
            "document",
            Json::Object(object(&[
                ("type", Json::Text("document_url".to_owned())),
                (
                    "document_url",
                    Json::Text("https://example.com/a.pdf".to_owned()),
                ),
            ])),
        ),
    ]));
    let document = body_document(&body).expect("the document is read");
    assert_eq!(document.source(), "https://example.com/a.pdf");
    let missing = Json::Object(object(&[("model", Json::Text("m".to_owned()))]));
    assert!(matches!(
        body_document(&missing),
        Err(OcrError::RequestField { path }) if path == "body.document"
    ));
}

struct SigningEnvironment {
    headers: Vec<(String, String)>,
    signer: RecordingSigner,
    region: String,
    service: String,
}

impl OcrEnvironment for SigningEnvironment {
    fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    fn signer(&self) -> Option<AwsSigningContext<'_>> {
        Some(AwsSigningContext {
            signer: &self.signer,
            region: &self.region,
            service: &self.service,
        })
    }
}

struct RecordingSigner;

#[async_trait]
impl AwsRequestSigner for RecordingSigner {
    async fn sign(
        &self,
        request: &AwsSigningRequest<'_>,
    ) -> Result<Vec<(String, String)>, OcrError> {
        assert_eq!(request.method, "POST");
        assert_eq!(request.service, "textract");
        assert_eq!(request.region, "us-east-1");
        assert!(request.body.windows(7).any(|window| window == b"\"model\""));
        Ok(vec![("x-amz-signature".to_owned(), "signed".to_owned())])
    }
}

#[tokio::test]
async fn signed_provider_requests_carry_the_signature_headers() {
    let request = prepared_request("http://example.com".to_owned(), Object::new());
    let environment = SigningEnvironment {
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        signer: RecordingSigner,
        region: "us-east-1".to_owned(),
        service: "textract".to_owned(),
    };
    let context = OcrCallContext {
        environment: &environment,
        hooks: &NoHooks,
    };
    let body = TestBody {
        model: "requested-model".to_owned(),
        document: provider_document(),
    };
    let outbound = transform_request_body(
        &TestConfig,
        &request,
        "http://example.com/v1/ocr",
        context,
        body,
    )
    .await
    .expect("the signed request builds");
    assert_eq!(
        outbound
            .headers
            .get("x-amz-signature")
            .and_then(|value| value.to_str().ok()),
        Some("signed")
    );
}
