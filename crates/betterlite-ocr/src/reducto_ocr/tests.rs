#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;
use uuid::Uuid;

use crate::json::{Json, Object};
use crate::ocr_config::OcrProviderConfig as _;
use crate::ocr_config::connection::{OcrConnection, OcrTransportConfig, ResolvedOcrCredentials};
use crate::ocr_config::{OcrRequestContext, OcrTransform, PreparedOcrRequest, Secrets};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::{OcrError, OcrHeaderError, OcrTransportError};
use crate::ocr_http::tests::support::{
    MockServer, delayed, object, ocr_client, prepared_request, reply,
};
use crate::ocr_http::{CallHooks, OcrWireRequest, ocr};
use crate::ocr_settings::OcrSettings;
use crate::request_format::RequestFormat;
use crate::response::OCRPage;

use super::ReductoProviderBody as _;
use super::{ReductoParseLegacyConfig, ReductoParseV3Config};

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

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

fn decimal(value: f64) -> Json {
    Json::Number(Number::from_f64(value).expect("the test number is finite"))
}

fn decode(bytes: &[u8]) -> Json {
    serde_json::from_slice(bytes).expect("the test JSON decodes")
}

fn document(source: &str) -> OcrDocument {
    OcrDocument::DocumentUrl {
        document_url: source.to_owned(),
        extra: BTreeMap::new(),
    }
}

fn pinned_boundary() -> Uuid {
    Uuid::from_u128(1)
}

fn v3() -> ReductoParseV3Config {
    ReductoParseV3Config {
        boundaries: pinned_boundary,
    }
}

fn legacy() -> ReductoParseLegacyConfig {
    ReductoParseLegacyConfig {
        boundaries: pinned_boundary,
    }
}

fn v3_request(api_base: String, optional_params: Object) -> PreparedOcrRequest {
    let mut request = prepared_request(api_base, optional_params);
    request.model = "parse-v3".to_owned();
    request.document = document("data:application/pdf;base64,YWJj");
    request.connection.api_key = Some(SecretString::from("test-key"));
    request
}

fn request_body(request: &str) -> Json {
    let (_, body) = request
        .split_once("\r\n\r\n")
        .expect("the request carries a body");
    decode(body.as_bytes())
}

fn block_values(page: &OCRPage) -> Vec<Json> {
    match page.extra.get("blocks") {
        Some(Json::List(blocks)) => blocks.clone(),
        Some(_) | None => Vec::new(),
    }
}

fn page_index_markdown(page: &OCRPage) -> Json {
    Json::Object(object(&[
        ("index", number(page.index)),
        ("markdown", text(&page.markdown)),
    ]))
}

#[tokio::test]
async fn data_uri_is_uploaded_once_and_the_parse_body_carries_the_file_id() {
    let upload = br#"{"file_id":"reducto://uploaded.pdf"}"#;
    let parsed = br#"{"usage":{"num_pages":1,"credits":2},"result":{"chunks":[{"content":"hello","blocks":[{"content":"hello","bbox":{"page":1}}]}]}}"#;
    let server = MockServer::start(vec![
        reply("200 OK", &[], upload),
        reply("200 OK", &[], parsed),
    ])
    .await;
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let request = v3_request(
        server.base.clone(),
        object(&[
            (
                "formatting",
                Json::Object(object(&[("table_output_format", text("html"))])),
            ),
            (
                "retrieval",
                Json::Object(object(&[("chunk_mode", text("section"))])),
            ),
            (
                "settings",
                Json::Object(object(&[("ocr_system", text("standard"))])),
            ),
        ]),
    );
    let response = ocr(&v3(), &client, &request, &NoHooks)
        .await
        .expect("the call succeeds");
    assert_eq!(response.pages.len(), 1);
    assert_eq!(response.pages[0].index, 0);
    assert_eq!(response.pages[0].markdown, "hello");
    let usage = response.usage_info.expect("usage is reported");
    assert_eq!(usage.pages_processed, Some(1));
    assert_eq!(usage.credits, Some(2.0_f64));

    let requests = server.requests().await;
    assert_eq!(requests.len(), 2);
    assert_upload_request(&requests[0]);
    assert_parse_request(&requests[1]);
}

fn assert_upload_request(upload_request: &str) {
    assert!(
        upload_request.starts_with("POST /upload HTTP/1.1"),
        "{upload_request}"
    );
    let lowered = upload_request.to_ascii_lowercase();
    assert!(
        lowered.contains("authorization: bearer test-key"),
        "{upload_request}"
    );
    assert!(
        lowered.contains("content-disposition: form-data; name=\"file\"; filename=\"document\""),
        "{upload_request}"
    );
    assert!(
        lowered.contains("content-type: application/pdf"),
        "{upload_request}"
    );
    assert!(
        lowered.contains("boundary=00000000000000000000000000000001"),
        "{upload_request}"
    );
    assert!(
        !lowered.contains("content-type: application/json"),
        "{upload_request}"
    );
    assert!(
        upload_request.contains("\r\n\r\nabc\r\n"),
        "{upload_request}"
    );
}

fn assert_parse_request(parse_request: &str) {
    assert!(
        parse_request.starts_with("POST /parse HTTP/1.1"),
        "{parse_request}"
    );
    assert_eq!(
        request_body(parse_request),
        Json::Object(object(&[
            ("input", text("reducto://uploaded.pdf")),
            (
                "formatting",
                Json::Object(object(&[("table_output_format", text("html"))]))
            ),
            (
                "retrieval",
                Json::Object(object(&[("chunk_mode", text("section"))]))
            ),
            (
                "settings",
                Json::Object(object(&[("ocr_system", text("standard"))]))
            ),
        ]))
    );
}

#[tokio::test]
async fn a_reducto_id_skips_the_upload_and_a_blank_one_is_a_request_field_error() {
    let parsed = br#"{"usage":{"num_pages":1},"chunks":[{"content":"kept"}]}"#;
    let server = MockServer::start(vec![reply("200 OK", &[], parsed)]).await;
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let mut request = v3_request(server.base.clone(), Object::new());
    request.document = document("reducto://already-uploaded.pdf");
    let response = ocr(&v3(), &client, &request, &NoHooks)
        .await
        .expect("the call succeeds");
    assert_eq!(response.pages[0].markdown, "kept");
    let requests = server.requests().await;
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].starts_with("POST /parse HTTP/1.1"),
        "{}",
        requests[0]
    );
    assert_eq!(
        request_body(&requests[0]),
        Json::Object(object(&[("input", text("reducto://already-uploaded.pdf"))]))
    );

    let blank = MockServer::start(Vec::new()).await;
    let mut blank_request = v3_request(blank.base.clone(), Object::new());
    blank_request.document = document("reducto://   ");
    let error = ocr(&v3(), &client, &blank_request, &NoHooks)
        .await
        .expect_err("a blank id fails");
    assert!(
        matches!(&error, OcrError::RequestField { path } if path == "document file id"),
        "{error:?}"
    );
    assert!(blank.requests().await.is_empty());
}

#[rstest]
#[case::http_url("https://example.com/document.pdf")]
#[case::unprefixed_text("not-a-document")]
#[tokio::test]
async fn a_source_that_is_neither_an_id_nor_a_data_uri_fails_before_a_request(
    #[case] source: &str,
) {
    let server = MockServer::start(Vec::new()).await;
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let mut request = v3_request(server.base.clone(), Object::new());
    request.document = document(source);
    let error = ocr(&v3(), &client, &request, &NoHooks)
        .await
        .expect_err("a plain URL is not a Reducto source");
    assert!(matches!(error, OcrError::ReductoSource), "{error:?}");
    assert!(server.requests().await.is_empty());
}

#[rstest]
#[case::no_comma("data:application/pdf")]
#[case::invalid_base64("data:application/pdf;base64,INVALID!")]
#[case::refused_mime_parameter(r#"data:application/pdf;x="";base64,YWJj"#)]
#[tokio::test]
async fn a_data_uri_the_parsers_refuse_is_invalid_data_uri(#[case] source: &str) {
    let server = MockServer::start(Vec::new()).await;
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let mut request = v3_request(server.base.clone(), Object::new());
    request.document = document(source);
    let error = ocr(&v3(), &client, &request, &NoHooks)
        .await
        .expect_err("the data URI is refused");
    assert!(matches!(error, OcrError::InvalidDataUri), "{error:?}");
    assert!(server.requests().await.is_empty());
}

#[tokio::test]
async fn a_data_uri_without_a_mime_type_sends_the_default_type() {
    let upload = br#"{"file_id":"reducto://uploaded.pdf"}"#;
    let parsed = br#"{"chunks":[{"content":"kept"}]}"#;
    let server = MockServer::start(vec![
        reply("200 OK", &[], upload),
        reply("200 OK", &[], parsed),
    ])
    .await;
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let mut request = v3_request(server.base.clone(), Object::new());
    request.document = document("data:;base64,YWJj");
    ocr(&v3(), &client, &request, &NoHooks)
        .await
        .expect("the call succeeds");
    let requests = server.requests().await;
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("content-type: text/plain;charset=us-ascii"),
        "{}",
        requests[0]
    );
}

#[tokio::test]
async fn v3_body_keeps_supported_options_and_drops_unknown_ones() {
    let config = v3();
    let selected = config
        .map_ocr_params(
            &object(&[
                ("formatting", Json::Null),
                ("settings", Json::Object(Object::new())),
                ("unknown", Json::Bool(true)),
            ]),
            "parse-v3",
        )
        .expect("the options map");
    assert_eq!(
        selected,
        object(&[
            ("formatting", Json::Null),
            ("settings", Json::Object(Object::new())),
        ])
    );
    assert_eq!(
        v3_body_for(&selected).await,
        Json::Object(object(&[
            ("input", text("reducto://ready.pdf")),
            ("formatting", Json::Null),
            ("settings", Json::Object(Object::new())),
        ]))
    );
}

#[test]
fn the_sync_transform_accepts_only_a_reducto_id() {
    let config = v3();
    let transform = OcrTransform {
        model: "parse-v3",
        optional_params: &Object::new(),
        headers: &[],
    };
    let body = config
        .transform_ocr_request(document("reducto://ready.pdf"), transform.clone())
        .expect("the id builds the body");
    assert_eq!(
        decode(&serde_json::to_vec(&body).expect("the body encodes")),
        Json::Object(object(&[("input", text("reducto://ready.pdf"))]))
    );
    let error = config
        .transform_ocr_request(
            document("data:application/pdf;base64,YWJj"),
            transform.clone(),
        )
        .expect_err("a data URI needs the async upload path");
    assert!(matches!(error, OcrError::ReductoSource), "{error:?}");
}

async fn v3_body_for(selected: &Object) -> Json {
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let connection = test_connection();
    let body = v3()
        .async_transform_ocr_request(
            document("reducto://ready.pdf"),
            OcrTransform {
                model: "parse-v3",
                optional_params: selected,
                headers: &[],
            },
            OcrRequestContext {
                client: &client,
                connection: &connection,
            },
        )
        .await
        .expect("the body builds");
    decode(&serde_json::to_vec(&body).expect("the body encodes"))
}

#[rstest]
#[case::present(
    object(&[
        ("enhance", Json::Object(object(&[("task", text("extract"))]))),
        ("unknown", Json::Bool(true)),
    ]),
    Json::Object(object(&[
        ("document_url", text("reducto://ready.pdf")),
        (
            "options",
            Json::Object(object(&[(
                "enhance",
                Json::Object(object(&[("task", text("extract"))])),
            )])),
        ),
    ]))
)]
#[case::explicit_null(
    object(&[("enhance", Json::Null), ("unknown", Json::Bool(true))]),
    Json::Object(object(&[("document_url", text("reducto://ready.pdf"))]))
)]
#[case::absent(
    object(&[("unknown", Json::Bool(true))]),
    Json::Object(object(&[("document_url", text("reducto://ready.pdf"))]))
)]
fn legacy_body_wraps_enhance_only_when_it_is_not_null(
    #[case] overrides: Object,
    #[case] expected: Json,
) {
    let selected = legacy()
        .map_ocr_params(&overrides, "parse-legacy")
        .expect("the options map");
    let body = ReductoParseLegacyConfig::body("reducto://ready.pdf".to_owned(), &selected);
    assert_eq!(
        decode(&serde_json::to_vec(&body).expect("the body encodes")),
        expected
    );
}

struct RecordingHook {
    seen: Arc<Mutex<Vec<Json>>>,
}

#[async_trait]
impl CallHooks for RecordingHook {
    async fn before_provider_request(
        &self,
        mut request: OcrWireRequest,
    ) -> Result<OcrWireRequest, OcrError> {
        self.seen
            .lock()
            .expect("the hook records")
            .push(request.body.clone());
        request
            .headers
            .push(("x-hook".to_owned(), "seen".to_owned()));
        request.body = Json::Object(object(&[
            ("type", text("document_url")),
            ("document_url", text("reducto://hooked.pdf")),
        ]));
        Ok(request)
    }

    async fn response_received(&self, _body: &[u8]) -> Result<(), OcrError> {
        Ok(())
    }
}

#[tokio::test]
async fn the_guardrail_sees_the_source_document_and_its_headers_reach_the_parse_call() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let hook = RecordingHook {
        seen: Arc::clone(&seen),
    };
    let parsed = br#"{"chunks":[{"content":"hooked"}]}"#;
    let server = MockServer::start(vec![reply("200 OK", &[], parsed)]).await;
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let request = v3_request(server.base.clone(), Object::new());
    let response = ocr(&v3(), &client, &request, &hook)
        .await
        .expect("the call succeeds");
    assert_eq!(response.pages[0].markdown, "hooked");
    {
        let bodies = seen.lock().expect("the hook recorded");
        assert_eq!(bodies.len(), 1);
        assert_eq!(
            bodies[0],
            Json::Object(object(&[
                ("type", text("document_url")),
                ("document_url", text("data:application/pdf;base64,YWJj")),
            ]))
        );
    };
    let requests = server.requests().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        request_body(&requests[0]),
        Json::Object(object(&[("input", text("reducto://hooked.pdf"))]))
    );
    assert!(
        requests[0].to_ascii_lowercase().contains("x-hook: seen"),
        "{}",
        requests[0]
    );
}

fn set_authorization(mut request: OcrWireRequest, value: &str) -> OcrWireRequest {
    request
        .headers
        .retain(|(name, _)| !name.eq_ignore_ascii_case("authorization"));
    request
        .headers
        .push(("authorization".to_owned(), value.to_owned()));
    request
}

struct HeaderHook;

#[async_trait]
impl CallHooks for HeaderHook {
    async fn before_provider_request(
        &self,
        request: OcrWireRequest,
    ) -> Result<OcrWireRequest, OcrError> {
        Ok(set_authorization(request, "Bearer hook-key"))
    }

    async fn response_received(&self, _body: &[u8]) -> Result<(), OcrError> {
        Ok(())
    }
}

#[rstest]
#[case::v3("parse-v3")]
#[case::legacy("parse-legacy")]
#[tokio::test]
async fn the_guardrail_headers_reach_both_the_upload_and_the_parse_call(#[case] model: &str) {
    let upload = br#"{"file_id":"reducto://uploaded.pdf"}"#;
    let parsed = br#"{"chunks":[{"content":"kept"}]}"#;
    let server = MockServer::start(vec![
        reply("200 OK", &[], upload),
        reply("200 OK", &[], parsed),
    ])
    .await;
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let mut request = v3_request(server.base.clone(), Object::new());
    request.model = model.to_owned();
    let response = if model == "parse-legacy" {
        ocr(&legacy(), &client, &request, &HeaderHook).await
    } else {
        ocr(&v3(), &client, &request, &HeaderHook).await
    }
    .expect("the call succeeds");
    assert_eq!(response.pages[0].markdown, "kept");
    let requests = server.requests().await;
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("authorization: bearer hook-key"),
        "{}",
        requests[0]
    );
    assert!(
        requests[1]
            .to_ascii_lowercase()
            .contains("authorization: bearer hook-key"),
        "{}",
        requests[1]
    );
}

struct ResponseHook {
    seen: Arc<Mutex<Vec<Vec<u8>>>>,
}

#[async_trait]
impl CallHooks for ResponseHook {
    async fn before_provider_request(
        &self,
        request: OcrWireRequest,
    ) -> Result<OcrWireRequest, OcrError> {
        Ok(request)
    }

    async fn response_received(&self, body: &[u8]) -> Result<(), OcrError> {
        self.seen
            .lock()
            .expect("the hook records")
            .push(body.to_vec());
        Ok(())
    }
}

#[tokio::test]
async fn the_response_hook_sees_the_parse_answer_once() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let hook = ResponseHook {
        seen: Arc::clone(&seen),
    };
    let upload = br#"{"file_id":"reducto://uploaded.pdf"}"#;
    let parsed = br#"{"chunks":[{"content":"kept"}]}"#;
    let server = MockServer::start(vec![
        reply("200 OK", &[], upload),
        reply("200 OK", &[], parsed),
    ])
    .await;
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let request = v3_request(server.base.clone(), Object::new());
    ocr(&v3(), &client, &request, &hook)
        .await
        .expect("the call succeeds");
    {
        let bodies = seen.lock().expect("the hook recorded");
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0], parsed.to_vec());
    };
    assert_eq!(server.requests().await.len(), 2);
}

#[tokio::test]
async fn upload_keeps_auth_headers_and_drops_content_type_and_content_length() {
    let upload = br#"{"file_id":"reducto://uploaded.pdf"}"#;
    let parsed = br#"{"chunks":[{"content":"kept"}]}"#;
    let server = MockServer::start(vec![
        reply("200 OK", &[], upload),
        reply("200 OK", &[], parsed),
    ])
    .await;
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let mut request = v3_request(server.base.clone(), Object::new());
    request.connection.extra_headers = vec![
        ("authorization".to_owned(), "Bearer supplied".to_owned()),
        ("content-type".to_owned(), "text/plain".to_owned()),
        ("content-length".to_owned(), "3".to_owned()),
        ("x-extra".to_owned(), "kept".to_owned()),
    ];
    ocr(&v3(), &client, &request, &NoHooks)
        .await
        .expect("the call succeeds");
    let requests = server.requests().await;
    assert_eq!(requests.len(), 2);
    let upload_request = &requests[0];
    let lowered = upload_request.to_ascii_lowercase();
    assert!(
        lowered.contains("authorization: bearer supplied"),
        "{upload_request}"
    );
    assert!(lowered.contains("x-extra: kept"), "{upload_request}");
    assert!(
        !lowered.contains("content-type: text/plain"),
        "{upload_request}"
    );
    assert!(!lowered.contains("content-length: 3"), "{upload_request}");
    assert!(
        lowered.contains("content-type: multipart/form-data"),
        "{upload_request}"
    );
}

#[tokio::test]
async fn an_upload_timeout_is_a_status_less_transport_failure() {
    let server = MockServer::start(vec![delayed(
        reply("200 OK", &[], br#"{"file_id":"reducto://uploaded.pdf"}"#),
        Duration::from_millis(300),
    )])
    .await;
    let client = ocr_client(OcrSettings::default(), Duration::from_millis(50));
    let request = v3_request(server.base.clone(), Object::new());
    let error = ocr(&v3(), &client, &request, &NoHooks)
        .await
        .expect_err("the stalled upload times out");
    assert!(
        matches!(error, OcrError::Transport(OcrTransportError::Failed(_))),
        "{error:?}"
    );
    assert_eq!(error.http_status_code(), None);
}

#[tokio::test]
async fn an_invalid_upload_header_is_a_request_field_error() {
    let server = MockServer::start(Vec::new()).await;
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let mut request = v3_request(server.base.clone(), Object::new());
    request.connection.extra_headers = vec![("bad header".to_owned(), "x".to_owned())];
    let error = ocr(&v3(), &client, &request, &NoHooks)
        .await
        .expect_err("the header is refused");
    assert!(
        matches!(error, OcrError::Headers(OcrHeaderError::Name { .. })),
        "{error:?}"
    );
    assert_eq!(error.http_status_code(), Some(400));
    assert!(server.requests().await.is_empty());
}

#[tokio::test]
async fn an_upload_failure_surfaces_the_provider_status_and_body() {
    let server = MockServer::start(vec![reply(
        "422 Unprocessable Entity",
        &[],
        b"the file was rejected",
    )])
    .await;
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let request = v3_request(server.base.clone(), Object::new());
    let error = ocr(&v3(), &client, &request, &NoHooks)
        .await
        .expect_err("an upload failure fails the call");
    assert!(
        matches!(
            &error,
            OcrError::Transport(OcrTransportError::Http { status: 422, body })
                if body == "the file was rejected"
        ),
        "{error:?}"
    );
}

#[tokio::test]
async fn an_upload_answer_without_a_file_id_fails_with_a_response_field() {
    let server = MockServer::start(vec![reply("200 OK", &[], br#"{"file_id":"  "}"#)]).await;
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let request = v3_request(server.base.clone(), Object::new());
    let error = ocr(&v3(), &client, &request, &NoHooks)
        .await
        .expect_err("a blank file id fails");
    assert!(
        matches!(&error, OcrError::ResponseField { path } if path == "file_id"),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_missing_key_and_a_blank_key_are_missing_reducto_api_key() {
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let mut request = prepared_request("https://platform.reducto.ai".to_owned(), Object::new());
    request.connection.api_key = None;
    let error = v3()
        .validate_environment(&request, &client)
        .await
        .expect_err("no key is an error");
    assert!(matches!(error, OcrError::MissingReductoApiKey), "{error:?}");

    request.connection.api_key = Some(SecretString::from("   "));
    let blank_error = v3()
        .validate_environment(&request, &client)
        .await
        .expect_err("a blank key falls through to the environment");
    assert!(
        matches!(blank_error, OcrError::MissingReductoApiKey),
        "{blank_error:?}"
    );
}

#[tokio::test]
async fn the_environment_key_is_used_when_the_request_has_none() {
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let mut request = prepared_request("https://platform.reducto.ai".to_owned(), Object::new());
    request.connection.api_key = None;
    let _previous = request.connection.secrets.insert(
        "REDUCTO_API_KEY".to_owned(),
        SecretString::from(" env-key "),
    );
    let headers = v3()
        .validate_environment(&request, &client)
        .await
        .expect("the environment key resolves");
    assert_eq!(
        headers,
        vec![("Authorization".to_owned(), "Bearer env-key".to_owned())]
    );
}

#[tokio::test]
async fn the_request_key_precedes_the_environment_key() {
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let mut request = prepared_request("https://platform.reducto.ai".to_owned(), Object::new());
    request.connection.api_key = Some(SecretString::from("request-key"));
    let _previous = request
        .connection
        .secrets
        .insert("REDUCTO_API_KEY".to_owned(), SecretString::from("env-key"));
    let headers = v3()
        .validate_environment(&request, &client)
        .await
        .expect("the request key resolves");
    assert_eq!(
        headers,
        vec![("Authorization".to_owned(), "Bearer request-key".to_owned())]
    );
}

#[tokio::test]
async fn a_supplied_authorization_header_wins_over_the_key_lookup() {
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let mut request = prepared_request("https://platform.reducto.ai".to_owned(), Object::new());
    request.connection.api_key = None;
    request.connection.extra_headers = vec![
        ("Authorization".to_owned(), "Bearer supplied".to_owned()),
        ("x-extra".to_owned(), "kept".to_owned()),
    ];
    let headers = v3()
        .validate_environment(&request, &client)
        .await
        .expect("a supplied header needs no key");
    assert_eq!(headers, request.connection.extra_headers);
}

#[rstest]
#[case::root("https://example.test", "https://example.test/parse")]
#[case::version_prefix("https://example.test/v2", "https://example.test/v2/parse")]
#[case::already_complete("https://example.test/parse", "https://example.test/parse")]
#[case::trailing_slash("https://example.test/v2/", "https://example.test/v2/parse")]
fn the_parse_url_completes_the_configured_base(#[case] api_base: &str, #[case] expected: &str) {
    let request = prepared_request(api_base.to_owned(), Object::new());
    let url = v3()
        .get_complete_url(&request, &Object::new(), &Vec::new())
        .expect("the URL builds");
    assert_eq!(url, expected);
}

#[test]
fn the_default_parse_url_is_the_reducto_platform() {
    let request = prepared_request(String::new(), Object::new());
    let url = v3()
        .get_complete_url(&request, &Object::new(), &Vec::new())
        .expect("the URL builds");
    assert_eq!(url, "https://platform.reducto.ai/parse");
}

#[rstest]
#[case::null_result(r#"{"result":null,"chunks":[{"content":"ignored"}]}"#, Vec::new())]
#[case::absent_result(
    r#"{"chunks":[{"content":"text"}]}"#,
    vec![Json::Object(object(&[("index", number(0)), ("markdown", text("text"))]))]
)]
#[case::object_result(
    r#"{"result":{"chunks":[{"content":"kept"}]}}"#,
    vec![Json::Object(object(&[("index", number(0)), ("markdown", text("kept"))]))]
)]
fn the_result_shapes_choose_their_own_pages(#[case] raw: &str, #[case] expected: Vec<Json>) {
    let response = v3()
        .transform_ocr_response("parse-v3", raw.as_bytes(), RequestFormat::Litellm)
        .expect("the response normalizes");
    let pages: Vec<Json> = response.pages.iter().map(page_index_markdown).collect();
    assert_eq!(pages, expected);
}

#[test]
fn pages_group_blocks_by_page_and_usage_accepts_lax_numbers() {
    let raw = br#"{"usage":{"num_pages":"2","credits":true},"result":{"type":"full","chunks":[{"blocks":[{"type":"Table","content":"B","bbox":{"left":0.1,"page":2,"original_page":4},"confidence":"high","image_url":null}]},{"blocks":[{"content":"A","bbox":{"page":1}},{"content":"C","bbox":{"page":1}}]}]}}"#;
    let response = v3()
        .transform_ocr_response("parse-v3", raw, RequestFormat::Litellm)
        .expect("the response normalizes");
    assert_eq!(response.model, "parse-v3");
    assert_eq!(response.pages.len(), 2);
    assert_eq!(response.pages[0].index, 0);
    assert_eq!(response.pages[0].markdown, "A\n\nC");
    assert_eq!(response.pages[1].index, 1);
    assert_eq!(response.pages[1].markdown, "B");
    assert_eq!(
        block_values(&response.pages[1]),
        vec![Json::Object(object(&[
            ("type", text("Table")),
            ("content", text("B")),
            (
                "bbox",
                Json::Object(object(&[
                    ("left", decimal(0.1)),
                    ("page", number(2)),
                    ("original_page", number(4)),
                ]))
            ),
            ("confidence", text("high")),
            ("image_url", Json::Null),
        ]))]
    );
    let usage = response.usage_info.expect("usage is reported");
    assert_eq!(usage.pages_processed, Some(2));
    assert_eq!(usage.credits, Some(1.0_f64));
}

#[test]
fn a_response_without_a_page_number_joins_the_chunk_content() {
    let raw = br#"{"chunks":[{"content":"one"},{"content":""},{"content":"two"}]}"#;
    let response = v3()
        .transform_ocr_response("parse-v3", raw, RequestFormat::Litellm)
        .expect("the response normalizes");
    assert_eq!(response.pages.len(), 1);
    assert_eq!(response.pages[0].index, 0);
    assert_eq!(response.pages[0].markdown, "one\n\ntwo");
    assert!(response.pages[0].extra.is_empty());
    assert!(response.usage_info.is_some());
}

#[test]
fn a_block_content_that_is_not_text_fails_with_a_response_field() {
    let raw = br#"{"result":{"chunks":[{"blocks":[{"content":5,"bbox":{"page":1}}]}]}}"#;
    let error = v3()
        .transform_ocr_response("parse-v3", raw, RequestFormat::Litellm)
        .expect_err("a non-text block content fails");
    assert!(
        matches!(&error, OcrError::ResponseField { path } if path == "result.chunks.blocks.content"),
        "{error:?}"
    );
}

#[test]
fn a_bad_block_page_number_leaves_the_block_out() {
    let raw = br#"{"result":{"chunks":[{"blocks":[{"content":"ignored","bbox":{"page":"invalid"}},{"content":"kept","bbox":{"page":2.5},"extra":null}]}]}}"#;
    let response = v3()
        .transform_ocr_response("parse-v3", raw, RequestFormat::Litellm)
        .expect("a bad page number is best effort");
    assert_eq!(response.pages.len(), 1);
    assert_eq!(response.pages[0].index, 1);
    assert_eq!(response.pages[0].markdown, "kept");
    assert_eq!(
        block_values(&response.pages[0]),
        vec![Json::Object(object(&[
            ("content", text("kept")),
            ("bbox", Json::Object(object(&[("page", decimal(2.5))])),),
            ("extra", Json::Null),
        ]))]
    );
}

#[test]
fn a_block_page_number_at_two_pow_63_saturates_at_i64_max() {
    let raw =
        br#"{"chunks":[{"blocks":[{"content":"kept","bbox":{"page":9223372036854775808.0}}]}]}"#;
    let response = v3()
        .transform_ocr_response("parse-v3", raw, RequestFormat::Litellm)
        .expect("the block page saturates");
    assert_eq!(response.pages.len(), 1);
    assert_eq!(response.pages[0].index, i64::MAX - 1);
    assert_eq!(response.pages[0].markdown, "kept");
}

#[rstest]
#[case::pages_integral_float(r#"{"usage":{"num_pages":2.0}}"#, Some(2), None)]
#[case::pages_numeric_string_with_fraction(r#"{"usage":{"num_pages":"2.0"}}"#, Some(2), None)]
#[case::pages_numeric_string_with_underscore(
    r#"{"usage":{"num_pages":"1_000"}}"#,
    Some(1000),
    None
)]
#[case::pages_boolean(r#"{"usage":{"num_pages":true}}"#, Some(1), None)]
#[case::credits_numeric_string(r#"{"usage":{"credits":"3"}}"#, None, Some(3.0_f64))]
#[case::credits_boolean(r#"{"usage":{"credits":true}}"#, None, Some(1.0_f64))]
fn usage_numbers_accept_the_forms_litellm_accepts(
    #[case] raw: &str,
    #[case] pages: Option<i64>,
    #[case] credits: Option<f64>,
) {
    let usage = v3()
        .transform_ocr_response("parse-v3", raw.as_bytes(), RequestFormat::Litellm)
        .expect("the usage parses")
        .usage_info
        .expect("usage is reported");
    assert_eq!(usage.pages_processed, pages);
    assert_eq!(usage.credits, credits);
}

#[rstest]
#[case::trimmed_string(
    r#"{"chunks":[{"blocks":[{"content":"a","bbox":{"page":" 3 "}}]}]}"#,
    2
)]
#[case::boolean(r#"{"chunks":[{"blocks":[{"content":"a","bbox":{"page":true}}]}]}"#, 0)]
#[case::zero(r#"{"chunks":[{"blocks":[{"content":"a","bbox":{"page":0}}]}]}"#, 0)]
#[case::negative(r#"{"chunks":[{"blocks":[{"content":"a","bbox":{"page":-1}}]}]}"#, 0)]
fn block_page_numbers_coerce_and_clamp(#[case] raw: &str, #[case] expected: i64) {
    let response = v3()
        .transform_ocr_response("parse-v3", raw.as_bytes(), RequestFormat::Litellm)
        .expect("the page parses");
    assert_eq!(response.pages.len(), 1);
    assert_eq!(response.pages[0].index, expected);
    assert_eq!(response.pages[0].markdown, "a");
}

#[rstest]
#[case::fractional_pages(r#"{"usage":{"num_pages":1.5}}"#)]
#[case::array_pages(r#"{"usage":{"num_pages":[]}}"#)]
#[case::object_credits(r#"{"usage":{"credits":{}}}"#)]
#[case::pages_over_i64(r#"{"usage":{"num_pages":9223372036854775808}}"#)]
#[case::infinite_credits(r#"{"usage":{"credits":"inf"}}"#)]
#[case::pages_exactly_two_pow_63(r#"{"usage":{"num_pages":9223372036854775808.0}}"#)]
#[case::text_credits(r#"{"usage":{"credits":"much"}}"#)]
fn a_usage_value_of_the_wrong_type_fails_with_a_response_field(#[case] raw: &str) {
    let error = v3()
        .transform_ocr_response("parse-v3", raw.as_bytes(), RequestFormat::Litellm)
        .expect_err("a mistyped usage value fails");
    assert!(matches!(error, OcrError::ResponseField { .. }), "{error:?}");
}

#[test]
fn the_native_request_format_keeps_the_provider_json() {
    let raw = br#"{"chunks":[{"content":"text"}],"job_id":"job_1"}"#;
    let response = v3()
        .transform_ocr_response("parse-v3", raw, RequestFormat::Native)
        .expect("the response normalizes");
    assert_eq!(response.provider_native_response, Some(decode(raw)));
}

fn test_connection() -> OcrConnection {
    OcrConnection::new(
        ResolvedOcrCredentials::default(),
        OcrTransportConfig::default(),
        OcrSettings::default(),
        Secrets::new(),
    )
}
