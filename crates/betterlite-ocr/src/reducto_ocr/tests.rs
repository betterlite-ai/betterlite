#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;

use crate::json::{Json, Object};
use crate::ocr_config::OcrProviderConfig as _;
use crate::ocr_config::connection::{OcrConnection, OcrTransportConfig, ResolvedOcrCredentials};
use crate::ocr_config::{OcrRequestContext, OcrTransform, PreparedOcrRequest, Secrets};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::{OcrError, OcrTransportError};
use crate::ocr_http::tests::support::{MockServer, object, ocr_client, prepared_request, reply};
use crate::ocr_http::{CallHooks, OcrWireRequest, ocr};
use crate::ocr_settings::OcrSettings;
use crate::request_format::RequestFormat;
use crate::response::OCRPage;

use super::request::legacy_body;
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
    let response = ocr(&ReductoParseV3Config, &client, &request, &NoHooks)
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
    let response = ocr(&ReductoParseV3Config, &client, &request, &NoHooks)
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
    let error = ocr(&ReductoParseV3Config, &client, &blank_request, &NoHooks)
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
    let error = ocr(&ReductoParseV3Config, &client, &request, &NoHooks)
        .await
        .expect_err("a plain URL is not a Reducto source");
    assert!(matches!(error, OcrError::ReductoSource), "{error:?}");
    assert!(server.requests().await.is_empty());
}

#[tokio::test]
async fn a_malformed_data_uri_is_invalid_data_uri() {
    let server = MockServer::start(Vec::new()).await;
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let mut request = v3_request(server.base.clone(), Object::new());
    request.document = document("data:application/pdf");
    let error = ocr(&ReductoParseV3Config, &client, &request, &NoHooks)
        .await
        .expect_err("a data URI without a comma fails");
    assert!(matches!(error, OcrError::InvalidDataUri), "{error:?}");
    assert!(server.requests().await.is_empty());
}

#[tokio::test]
async fn v3_body_keeps_supported_options_and_drops_unknown_ones() {
    let config = ReductoParseV3Config;
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

async fn v3_body_for(selected: &Object) -> Json {
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let connection = test_connection();
    let body = ReductoParseV3Config
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
    let selected = ReductoParseLegacyConfig
        .map_ocr_params(&overrides, "parse-legacy")
        .expect("the options map");
    let body = legacy_body("reducto://ready.pdf".to_owned(), &selected);
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
        let Json::Object(body) = &mut request.body else {
            return Err(OcrError::RequestField {
                path: "guardrail.body".to_owned(),
            });
        };
        let hooked = object(&[
            ("type", text("document_url")),
            ("document_url", text("reducto://hooked.pdf")),
        ]);
        let _previous = body.insert("document".to_owned(), Json::Object(hooked));
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
    let response = ocr(&ReductoParseV3Config, &client, &request, &hook)
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
    ocr(&ReductoParseV3Config, &client, &request, &NoHooks)
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
async fn an_upload_failure_surfaces_the_provider_status_and_body() {
    let server = MockServer::start(vec![reply(
        "422 Unprocessable Entity",
        &[],
        b"the file was rejected",
    )])
    .await;
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let request = v3_request(server.base.clone(), Object::new());
    let error = ocr(&ReductoParseV3Config, &client, &request, &NoHooks)
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
    let error = ocr(&ReductoParseV3Config, &client, &request, &NoHooks)
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
    let error = ReductoParseV3Config
        .validate_environment(&request, &client)
        .await
        .expect_err("no key is an error");
    assert!(matches!(error, OcrError::MissingReductoApiKey), "{error:?}");

    request.connection.api_key = Some(SecretString::from("   "));
    let blank_error = ReductoParseV3Config
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
    let headers = ReductoParseV3Config
        .validate_environment(&request, &client)
        .await
        .expect("the environment key resolves");
    assert_eq!(
        headers,
        vec![("Authorization".to_owned(), "Bearer env-key".to_owned())]
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
    let headers = ReductoParseV3Config
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
    let url = ReductoParseV3Config
        .get_complete_url(&request, &Object::new(), &Vec::new())
        .expect("the URL builds");
    assert_eq!(url, expected);
}

#[test]
fn the_default_parse_url_is_the_reducto_platform() {
    let request = prepared_request(String::new(), Object::new());
    let url = ReductoParseV3Config
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
    let response = ReductoParseV3Config
        .transform_ocr_response("parse-v3", raw.as_bytes(), RequestFormat::Litellm)
        .expect("the response normalizes");
    let pages: Vec<Json> = response.pages.iter().map(page_index_markdown).collect();
    assert_eq!(pages, expected);
}

#[test]
fn pages_group_blocks_by_page_and_usage_accepts_lax_numbers() {
    let raw = br#"{"usage":{"num_pages":"2","credits":true},"result":{"type":"full","chunks":[{"blocks":[{"type":"Table","content":"B","bbox":{"left":0.1,"page":2,"original_page":4},"confidence":"high","image_url":null}]},{"blocks":[{"content":"A","bbox":{"page":1}},{"content":"C","bbox":{"page":1}}]}]}}"#;
    let response = ReductoParseV3Config
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
    let response = ReductoParseV3Config
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
    let error = ReductoParseV3Config
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
    let response = ReductoParseV3Config
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

#[rstest]
#[case::fractional_pages(r#"{"usage":{"num_pages":1.5}}"#)]
#[case::array_pages(r#"{"usage":{"num_pages":[]}}"#)]
#[case::object_credits(r#"{"usage":{"credits":{}}}"#)]
#[case::text_credits(r#"{"usage":{"credits":"much"}}"#)]
fn a_usage_value_of_the_wrong_type_fails_with_a_response_field(#[case] raw: &str) {
    let error = ReductoParseV3Config
        .transform_ocr_response("parse-v3", raw.as_bytes(), RequestFormat::Litellm)
        .expect_err("a mistyped usage value fails");
    assert!(matches!(error, OcrError::ResponseField { .. }), "{error:?}");
}

#[test]
fn the_native_request_format_keeps_the_provider_json() {
    let raw = br#"{"chunks":[{"content":"text"}],"job_id":"job_1"}"#;
    let response = ReductoParseV3Config
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
