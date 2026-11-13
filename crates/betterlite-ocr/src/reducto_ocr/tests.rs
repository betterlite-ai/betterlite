#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use rstest::rstest;
use secrecy::SecretString;
use tokio::time::timeout;
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

mod response;

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

async fn recorded(server: MockServer) -> Vec<String> {
    timeout(Duration::from_secs(5), server.requests())
        .await
        .expect("the provider saw every request it was owed")
}

fn request_body(request: &str) -> Json {
    let (_, body) = request
        .split_once("\r\n\r\n")
        .expect("the request carries a body");
    decode(body.as_bytes())
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

    let requests = recorded(server).await;
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
    let requests = recorded(server).await;
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
    assert!(recorded(blank).await.is_empty());
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
    assert!(recorded(server).await.is_empty());
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
    assert!(recorded(server).await.is_empty());
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
    let requests = recorded(server).await;
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0].contains("Content-Type: text/plain;charset=us-ascii"),
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
    let requests = recorded(server).await;
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
    let requests = recorded(server).await;
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
    assert_eq!(recorded(server).await.len(), 2);
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
    let requests = recorded(server).await;
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
    assert!(recorded(server).await.is_empty());
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

#[test]
fn both_configs_name_the_reducto_api_key_secret() {
    assert_eq!(v3().secret_names(), vec!["REDUCTO_API_KEY"]);
    assert_eq!(legacy().secret_names(), vec!["REDUCTO_API_KEY"]);
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

fn test_connection() -> OcrConnection {
    OcrConnection::new(
        ResolvedOcrCredentials::default(),
        OcrTransportConfig::default(),
        OcrSettings::default(),
        Secrets::new(),
    )
}
