#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::Number;

use crate::json::{Json, Object};
use crate::ocr_config::connection::{
    OcrConnection, OcrInputSource, OcrSourced, OcrTransportConfig, ResolvedOcrCredentials,
};
use crate::ocr_config::{
    OCR_INLINE_MAX_BYTES, OCR_RESPONSE_MAX_BYTES, OcrProviderConfig as _, OcrTransform,
    PreparedOcrRequest, Secrets,
};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::{OcrAuthError, OcrError};
use crate::ocr_http::tests::support::{MockServer, object, ocr_client, reply};
use crate::ocr_http::{CallHooks, OcrWireRequest, ocr};
use crate::ocr_settings::OcrSettings;
use crate::request_format::RequestFormat;
use crate::response::OCRResponse;

use super::options::{CohereOptions, OutputFormat};
use super::{COHERE_API_KEY_ENV, CohereParseConfig, validate_document};

fn image_document(source: &str) -> OcrDocument {
    OcrDocument::ImageUrl {
        image_url: source.to_owned(),
        extra: BTreeMap::new(),
    }
}

fn document_url(source: &str) -> OcrDocument {
    OcrDocument::DocumentUrl {
        document_url: source.to_owned(),
        extra: BTreeMap::new(),
    }
}

fn connection(
    api_key: Option<&str>,
    env_key: Option<&str>,
    extra_headers: Vec<(String, String)>,
    api_base: Option<&str>,
) -> OcrConnection {
    let mut secrets = Secrets::new();
    if let Some(value) = env_key {
        let _previous = secrets.insert(
            COHERE_API_KEY_ENV.to_owned(),
            SecretString::from(value.to_owned()),
        );
    }
    OcrConnection::new(
        ResolvedOcrCredentials {
            api_key: api_key.map(|value| OcrSourced {
                value: SecretString::from(value.to_owned()),
                source: OcrInputSource::Request,
            }),
            api_base: api_base.map(|value| OcrSourced {
                value: value.to_owned(),
                source: OcrInputSource::Request,
            }),
        },
        OcrTransportConfig {
            extra_headers,
            extra_headers_source: OcrInputSource::Request,
            timeout: None,
            max_response_bytes: OCR_RESPONSE_MAX_BYTES,
        },
        OcrSettings::default(),
        secrets,
    )
}

fn prepared(
    document: OcrDocument,
    optional_params: Object,
    connection: OcrConnection,
) -> PreparedOcrRequest {
    PreparedOcrRequest {
        model: "parse".to_owned(),
        document,
        connection,
        caller_document: false,
        optional_params,
        input_sources: BTreeMap::new(),
        azure_ad_token_provider: None,
    }
}

async fn environment(connection: OcrConnection) -> Result<Vec<(String, String)>, OcrError> {
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let request = prepared(
        image_document("https://example.com/a.png"),
        Object::new(),
        connection,
    );
    CohereParseConfig
        .validate_environment(&request, &client)
        .await
}

fn normalize(raw: &str) -> Result<OCRResponse, OcrError> {
    CohereParseConfig.transform_ocr_response("parse-v5.0", raw.as_bytes(), RequestFormat::Litellm)
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

async fn call_once() -> (OCRResponse, Vec<String>) {
    let body = br#"{"pages":[{"index":0,"markdown":{"content":"hello","images":[{"bounding_box":{"top_left_x":1},"category":"logo"}]}}],"meta":{"billed_units":{"pages":2}}}"#;
    let server = MockServer::start(vec![reply("200 OK", &[], body)]).await;
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let params = object(&[
        ("output_format", Json::Text("markdown".to_owned())),
        ("req_format", Json::Text("litellm".to_owned())),
    ]);
    let request = prepared(
        image_document("data:image/png;base64,YWJj"),
        params,
        connection(
            Some("request-key"),
            Some("env-key"),
            Vec::new(),
            Some(&server.base),
        ),
    );
    let response = ocr(&CohereParseConfig, &client, &request, &NoHooks)
        .await
        .expect("the call succeeds");
    (response, server.requests().await)
}

#[tokio::test]
async fn sends_the_parse_request_with_the_bearer_key_and_body() {
    let (_, requests) = call_once().await;
    assert_eq!(requests.len(), 1);
    let sent = &requests[0];
    assert!(sent.starts_with("POST /v2/parse HTTP/1.1"), "{sent}");
    let lowered = sent.to_ascii_lowercase();
    assert!(
        lowered.contains("authorization: bearer request-key"),
        "{sent}"
    );
    assert!(lowered.contains("content-type: application/json"), "{sent}");
    assert!(sent.contains("\"model\":\"parse\""), "{sent}");
    assert!(sent.contains("\"output_format\":\"markdown\""), "{sent}");
    assert!(
        sent.contains(
            "\"document\":{\"type\":\"image_url\",\"image_url\":\"data:image/png;base64,YWJj\"}"
        ),
        "{sent}"
    );
    assert!(!sent.contains("req_format"), "{sent}");
}

#[tokio::test]
async fn normalizes_the_provider_answer() {
    let (response, _) = call_once().await;
    assert_eq!(response.model, "parse");
    assert_eq!(response.pages.len(), 1);
    assert_eq!(response.pages[0].index, 0);
    assert_eq!(response.pages[0].markdown, "hello");
    let image = response.pages[0]
        .images
        .as_ref()
        .expect("the markdown image is kept");
    assert_eq!(
        image[0]
            .bbox
            .as_ref()
            .and_then(|bbox| bbox.get("top_left_x")),
        Some(&Json::Number(Number::from(1_i64)))
    );
    assert_eq!(
        response
            .usage_info
            .as_ref()
            .and_then(|usage| usage.pages_processed),
        Some(2)
    );
}

#[tokio::test]
async fn the_request_key_wins_over_the_environment_key() {
    let headers = environment(connection(
        Some("request-key"),
        Some("env-key"),
        Vec::new(),
        None,
    ))
    .await
    .expect("the key resolves");
    assert_eq!(
        headers,
        vec![("Authorization".to_owned(), "Bearer request-key".to_owned())]
    );
}

#[tokio::test]
async fn the_environment_key_is_used_without_a_request_key() {
    let headers = environment(connection(None, Some("env-key"), Vec::new(), None))
        .await
        .expect("the key resolves");
    assert_eq!(
        headers,
        vec![("Authorization".to_owned(), "Bearer env-key".to_owned())]
    );
}

#[tokio::test]
async fn a_supplied_authorization_header_wins_over_both_keys() {
    let extra = vec![("Authorization".to_owned(), "Bearer supplied".to_owned())];
    let headers = environment(connection(
        Some("request-key"),
        Some("env-key"),
        extra.clone(),
        None,
    ))
    .await
    .expect("the supplied header wins");
    assert_eq!(headers, extra);
}

#[tokio::test]
async fn a_blank_request_key_falls_back_to_the_environment_key() {
    let headers = environment(connection(Some("  "), Some("env-key"), Vec::new(), None))
        .await
        .expect("the environment key is used");
    assert_eq!(
        headers,
        vec![("Authorization".to_owned(), "Bearer env-key".to_owned())]
    );
}

#[tokio::test]
async fn extra_headers_are_sent_after_the_generated_authorization() {
    let extra = vec![("X-Tenant".to_owned(), "tenant-a".to_owned())];
    let headers = environment(connection(Some("request-key"), None, extra, None))
        .await
        .expect("the key resolves");
    assert_eq!(
        headers,
        vec![
            ("Authorization".to_owned(), "Bearer request-key".to_owned()),
            ("X-Tenant".to_owned(), "tenant-a".to_owned()),
        ]
    );
}

#[tokio::test]
async fn the_environment_key_keeps_the_spaces_it_was_read_with() {
    let headers = environment(connection(None, Some(" env-key "), Vec::new(), None))
        .await
        .expect("the key resolves");
    assert_eq!(
        headers,
        vec![("Authorization".to_owned(), "Bearer  env-key ".to_owned())]
    );
}

#[tokio::test]
async fn a_blank_environment_key_is_the_missing_key_error() {
    let error = environment(connection(None, Some("  "), Vec::new(), None))
        .await
        .expect_err("a blank environment key fails");
    assert!(matches!(
        error,
        OcrError::Auth(OcrAuthError::MissingApiKey { .. })
    ));
}

#[tokio::test]
async fn a_missing_key_is_the_cohere_authentication_error() {
    let error = environment(connection(None, None, Vec::new(), None))
        .await
        .expect_err("no key fails");
    let OcrError::Auth(auth) = &error else {
        panic!("the missing key is the auth error");
    };
    let OcrAuthError::MissingApiKey {
        provider,
        environment_variable,
    } = auth
    else {
        panic!("the auth error names the missing key");
    };
    assert_eq!(*provider, "Cohere");
    assert_eq!(*environment_variable, COHERE_API_KEY_ENV);
    let message = auth.to_string();
    assert!(message.contains("COHERE_API_KEY"), "{message}");
    assert!(message.contains("pass api_key"), "{message}");
}

#[rstest]
#[case::document_url(document_url("https://example.com/a.pdf"))]
#[case::pdf_data_uri(image_document("data:application/pdf;base64,YWJj"))]
#[case::empty_image_url(image_document(""))]
#[case::document_data_uri(document_url("data:image/png;base64,YWJj"))]
fn cohere_parse_accepts_only_images(#[case] document: OcrDocument) {
    assert!(matches!(
        validate_document(&document),
        Err(OcrError::CohereImageOnly)
    ));
}

#[rstest]
#[case::remote("https://example.com/scan.png")]
#[case::inline_png("data:image/png;base64,YWJj")]
#[case::uppercase_data_uri("DATA:image/jpeg;BASE64,YWJj")]
fn accepts_image_sources(#[case] source: &str) {
    validate_document(&image_document(source)).expect("the image is accepted");
}

#[test]
fn a_malformed_data_uri_is_not_an_image_only_error() {
    assert!(matches!(
        validate_document(&image_document("data:image/png;base64")),
        Err(OcrError::InvalidDataUri)
    ));
}

#[test]
fn an_image_data_uri_with_invalid_base64_is_rejected() {
    assert!(matches!(
        validate_document(&image_document("data:image/png;base64,@@@")),
        Err(OcrError::InvalidDataUri)
    ));
}

#[test]
fn an_image_data_uri_over_the_inline_cap_is_rejected() {
    let groups = OCR_INLINE_MAX_BYTES.saturating_add(1).div_ceil(3);
    let encoded = "A".repeat(groups.saturating_mul(4));
    let source = format!("data:image/png;base64,{encoded}");
    assert!(matches!(
        validate_document(&image_document(&source)),
        Err(OcrError::InlineDocumentTooLarge)
    ));
}

#[test]
fn the_health_check_document_is_the_png_data_uri() {
    let OcrDocument::ImageUrl { image_url, extra } = CohereParseConfig.get_health_check_document()
    else {
        panic!("the health check document is an image");
    };
    assert_eq!(
        image_url,
        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4//8/AAX+Av4N70a4AAAAAElFTkSuQmCC"
    );
    assert!(extra.is_empty());
    validate_document(&OcrDocument::ImageUrl { image_url, extra })
        .expect("the health check document is valid");
}

#[rstest]
#[case::bare("", "https://example.com/v2/parse?tenant=a")]
#[case::version("/v2", "https://example.com/v2/parse?tenant=a")]
#[case::complete("/v2/parse", "https://example.com/v2/parse?tenant=a")]
#[case::proxy_prefix("/cohere/", "https://example.com/cohere/v2/parse?tenant=a")]
#[case::proxy_version("/cohere/v2", "https://example.com/cohere/v2/parse?tenant=a")]
fn completes_the_parse_url_without_duplicate_paths(#[case] suffix: &str, #[case] expected: &str) {
    let base = format!("https://example.com{suffix}?tenant=a");
    let request = prepared(
        image_document("https://example.com/a.png"),
        Object::new(),
        connection(None, Some("key"), Vec::new(), Some(&base)),
    );
    let url = CohereParseConfig
        .get_complete_url(&request, &CohereOptions::default(), &Vec::new())
        .expect("the base completes");
    assert_eq!(url, expected);
}

#[test]
fn the_default_base_is_the_cohere_api() {
    let request = prepared(
        image_document("https://example.com/a.png"),
        Object::new(),
        connection(None, Some("key"), Vec::new(), None),
    );
    let url = CohereParseConfig
        .get_complete_url(&request, &CohereOptions::default(), &Vec::new())
        .expect("the default base completes");
    assert_eq!(url, "https://api.cohere.com/v2/parse");
}

#[rstest]
#[case::relative("relative/path")]
#[case::unsupported_scheme("ftp://example.com")]
#[case::empty("")]
fn rejects_non_http_api_bases(#[case] api_base: &str) {
    let request = prepared(
        image_document("https://example.com/a.png"),
        Object::new(),
        connection(None, Some("key"), Vec::new(), Some(api_base)),
    );
    let error = CohereParseConfig
        .get_complete_url(&request, &CohereOptions::default(), &Vec::new())
        .expect_err("the base fails");
    assert!(matches!(error, OcrError::RequestField { path } if path == "api_base"));
}

#[test]
fn options_map_the_supported_fields_and_ignore_the_rest() {
    let params = object(&[
        ("output_format", Json::Text("blocks".to_owned())),
        ("req_format", Json::Text("native".to_owned())),
        ("extension", Json::Bool(false)),
    ]);
    let options = CohereParseConfig
        .map_ocr_params(&params, "parse")
        .expect("the options map");
    assert_eq!(options.output_format, Some(OutputFormat::Blocks));
}

#[test]
fn an_unknown_output_format_names_the_option() {
    let params =
        serde_json::from_str::<Object>(r#"{"output_format":"html"}"#).expect("the options parse");
    let error = CohereParseConfig
        .map_ocr_params(&params, "parse")
        .expect_err("html is not an output format");
    assert!(matches!(
        error,
        OcrError::RequestField { path } if path == "optional_params.output_format"
    ));
}

#[rstest]
#[case::absent("{}")]
#[case::null(r#"{"output_format":null}"#)]
fn an_absent_or_null_output_format_defaults_to_markdown(#[case] raw: &str) {
    let params = serde_json::from_str::<Object>(raw).expect("the options parse");
    let options = CohereParseConfig
        .map_ocr_params(&params, "parse")
        .expect("the options map");
    assert_eq!(options.output_format, None);
}

#[rstest]
#[case::defaults(
    CohereOptions::default(),
    r#"{"model":"parse-v5.0","document":{"type":"image_url","image_url":"https://example.com/a.png"},"output_format":"markdown"}"#
)]
#[case::blocks(
    CohereOptions { output_format: Some(OutputFormat::Blocks) },
    r#"{"model":"parse-v5.0","document":{"type":"image_url","image_url":"https://example.com/a.png"},"output_format":"blocks"}"#
)]
fn builds_the_parse_body(#[case] params: CohereOptions, #[case] expected: &str) {
    let transform = OcrTransform {
        model: "parse-v5.0",
        optional_params: &params,
        headers: &[],
    };
    let body = CohereParseConfig
        .transform_ocr_request(image_document("https://example.com/a.png"), transform)
        .expect("the body builds");
    let serialized = serde_json::to_string(&body).expect("the body serializes");
    assert_eq!(serialized, expected);
}

#[test]
fn a_non_image_document_builds_no_body() {
    let params = CohereOptions::default();
    let transform = OcrTransform {
        model: "parse-v5.0",
        optional_params: &params,
        headers: &[],
    };
    assert!(matches!(
        CohereParseConfig
            .transform_ocr_request(document_url("https://example.com/a.pdf"), transform),
        Err(OcrError::CohereImageOnly)
    ));
}

const RICH_RESPONSE: &str = r#"{
    "pages": [
        {"type":"markdown","index":4,"markdown":{"content":"receipt","images":[
            {"id":"image","bounding_box":{"top_left_x":1,"top_left_y":2},"description":"scan","category":"logo"}
        ]}},
        {"type":"blocks","blocks":[{"type":"text","text":{"content":"total"}}]}
    ],
    "meta":{"api_version":{"version":"2"},"billed_units":{"pages":3}}
}"#;

fn rich_response() -> OCRResponse {
    normalize(RICH_RESPONSE).expect("the response normalizes")
}

#[test]
fn keeps_the_page_index_markdown_and_billed_pages() {
    let response = rich_response();
    assert_eq!(response.model, "parse-v5.0");
    assert_eq!(response.object, "ocr");
    assert_eq!(response.pages.len(), 2);
    assert_eq!(response.pages[0].index, 4);
    assert_eq!(response.pages[0].markdown, "receipt");
    assert_eq!(response.pages[1].index, 1);
    assert_eq!(response.pages[1].markdown, "");
    assert_eq!(response.pages[1].images, None);
    assert_eq!(
        response
            .usage_info
            .as_ref()
            .and_then(|usage| usage.pages_processed),
        Some(3)
    );
}

#[test]
fn copies_bounding_boxes_and_keeps_page_blocks() {
    let response = rich_response();
    let image = &response.pages[0]
        .images
        .as_ref()
        .expect("the image is kept")[0];
    assert_eq!(
        image.bbox.as_ref().and_then(|bbox| bbox.get("top_left_x")),
        Some(&Json::Number(Number::from(1_i64)))
    );
    assert!(image.extra.contains_key("bounding_box"));
    assert_eq!(
        image.extra.get("description"),
        Some(&Json::Text("scan".to_owned()))
    );
    assert_eq!(
        image.extra.get("category"),
        Some(&Json::Text("logo".to_owned()))
    );
    let blocks = serde_json::from_str::<Json>(r#"[{"type":"text","text":{"content":"total"}}]"#)
        .expect("the blocks parse");
    assert_eq!(response.pages[1].extra.get("blocks"), Some(&blocks));
}

#[rstest]
#[case::integer(r#"{"pages":[{"index":7}]}"#, 7)]
#[case::numeric_string(r#"{"pages":[{"index":"2"}]}"#, 2)]
#[case::integral_float(r#"{"pages":[{"index":1.0}]}"#, 1)]
#[case::boolean(r#"{"pages":[{"index":true}]}"#, 1)]
#[case::missing(r#"{"pages":[{}]}"#, 0)]
fn page_index_uses_the_lenient_value_or_the_position(#[case] raw: &str, #[case] expected: i64) {
    let response = normalize(raw).expect("the response normalizes");
    assert_eq!(response.pages[0].index, expected);
}

#[test]
fn a_missing_index_uses_the_page_position() {
    let response = normalize(r#"{"pages":[{},{}]}"#).expect("the response normalizes");
    assert_eq!(response.pages[1].index, 1);
}

#[rstest]
#[case::plain("2", Some(2))]
#[case::signed_plus("+4", Some(4))]
#[case::signed_minus("-3", Some(-3))]
#[case::underscores("1_000", Some(1000))]
#[case::whole_fraction("1.00", Some(1))]
#[case::underscored_fraction("1_0.00", Some(10))]
#[case::empty_fraction("1.", None)]
#[case::fractional("1.5", None)]
#[case::leading_underscore("_1", None)]
#[case::trailing_underscore("1_", None)]
#[case::double_underscore("1__0", None)]
#[case::signed_leading_underscore("+_1", None)]
#[case::trailing_letter("1a", None)]
#[case::empty("", None)]
#[case::sign_only("+", None)]
fn string_page_index_follows_the_lenient_integer_rule(
    #[case] index: &str,
    #[case] expected: Option<i64>,
) {
    let quoted = serde_json::to_string(index).expect("the index serializes");
    let raw = format!(r#"{{"pages":[{{"index":{quoted}}}]}}"#);
    match expected {
        Some(value) => {
            let response = normalize(&raw).expect("the string index decodes");
            assert_eq!(response.pages[0].index, value);
        }
        None => assert!(matches!(
            normalize(&raw),
            Err(OcrError::ResponseField { path }) if path == "pages[0].index"
        )),
    }
}

#[rstest]
#[case::billed_over_count(r#"{"pages":[{},{}],"meta":{"billed_units":{"pages":3}}}"#, 3)]
#[case::billed_zero_over_count(r#"{"pages":[{},{}],"meta":{"billed_units":{"pages":0}}}"#, 0)]
#[case::billed_float(r#"{"pages":[],"meta":{"billed_units":{"pages":1.0}}}"#, 1)]
#[case::missing_billed(r#"{"pages":[{},{}]}"#, 2)]
#[case::null_meta(r#"{"pages":[{}],"meta":null}"#, 1)]
#[case::null_billed_units(r#"{"pages":[{}],"meta":{"billed_units":null}}"#, 1)]
#[case::empty("{}", 0)]
fn usage_pages_processed_prefers_the_billed_count(#[case] raw: &str, #[case] expected: i64) {
    let response = normalize(raw).expect("the response normalizes");
    assert_eq!(
        response.usage_info.and_then(|usage| usage.pages_processed),
        Some(expected)
    );
}

#[rstest]
#[case::fractional_index(r#"{"pages":[{"index":1.5}]}"#, "pages[0].index")]
#[case::invalid_index(r#"{"pages":[{"index":"bad"}]}"#, "pages[0].index")]
#[case::null_pages(r#"{"pages":null}"#, "pages")]
#[case::invalid_markdown(r#"{"pages":[{"markdown":"text"}]}"#, "pages[0].markdown")]
#[case::invalid_blocks(r#"{"pages":[{"blocks":"x"}]}"#, "pages[0].blocks")]
#[case::fractional_billed(
    r#"{"pages":[],"meta":{"billed_units":{"pages":1.5}}}"#,
    "meta.billed_units.pages"
)]
fn malformed_responses_name_the_field(#[case] raw: &str, #[case] path: &str) {
    let error = normalize(raw).expect_err("the response fails");
    let OcrError::ResponseField { path: reported } = &error else {
        panic!("the response failure names a field");
    };
    assert_eq!(reported, path);
}

#[test]
fn a_malformed_image_field_names_its_path() {
    let error = normalize(r#"{"pages":[{"markdown":{"images":[{"image_base64":42}]}}]}"#)
        .expect_err("the image fails");
    let OcrError::ResponseField { path } = &error else {
        panic!("the image failure names a field");
    };
    assert!(path.starts_with("pages[0].markdown.images[0]"), "{path}");
}

#[test]
fn empty_images_and_missing_markdown_use_the_page_defaults() {
    let response =
        normalize(r#"{"pages":[{"markdown":{"content":"text","images":[]}},{"markdown":null}]}"#)
            .expect("the response normalizes");
    assert_eq!(response.pages[0].markdown, "text");
    assert_eq!(response.pages[0].images, None);
    assert_eq!(response.pages[1].markdown, "");
    assert_eq!(response.pages[1].images, None);
}

#[test]
fn an_image_without_a_bounding_box_has_no_bbox() {
    let response =
        normalize(r#"{"pages":[{"markdown":{"images":[{"image_base64":"aGVsbG8="}]}}]}"#)
            .expect("the response normalizes");
    let image = &response.pages[0]
        .images
        .as_ref()
        .expect("the image is kept")[0];
    assert_eq!(image.image_base64.as_deref(), Some("aGVsbG8="));
    assert_eq!(image.bbox, None);
}

#[test]
fn a_non_object_bounding_box_is_not_copied_to_bbox() {
    let response = normalize(r#"{"pages":[{"markdown":{"images":[{"bounding_box":[1,2]}]}}]}"#)
        .expect("the image decodes");
    let image = &response.pages[0]
        .images
        .as_ref()
        .expect("the image is kept")[0];
    assert_eq!(image.bbox, None);
    let expected = serde_json::from_str::<Json>("[1,2]").expect("the bounding box parses");
    assert_eq!(image.extra.get("bounding_box"), Some(&expected));
}

#[test]
fn a_provider_bbox_is_replaced_by_the_bounding_box() {
    let response =
        normalize(r#"{"pages":[{"markdown":{"images":[{"bounding_box":{"x":1},"bbox":"old"}]}}]}"#)
            .expect("the image decodes");
    let image = &response.pages[0]
        .images
        .as_ref()
        .expect("the image is kept")[0];
    let expected = serde_json::from_str::<Object>(r#"{"x":1}"#).expect("the box parses");
    assert_eq!(image.bbox, Some(expected));
}

#[test]
fn usage_info_sets_only_the_pages_processed() {
    let usage = normalize(r#"{"pages":[]}"#)
        .expect("the response normalizes")
        .usage_info
        .expect("the usage is set");
    assert_eq!(usage.pages_processed, Some(0));
    assert_eq!(usage.pages_processed_annotation, None);
    assert_eq!(usage.credits, None);
    assert_eq!(usage.doc_size_bytes, None);
    assert!(usage.extra.is_empty());
}

#[test]
fn native_format_keeps_the_provider_json_and_litellm_does_not() {
    let raw = r#"{"pages":[{"index":0}],"extra":{"kept":true}}"#;
    let native = CohereParseConfig
        .transform_ocr_response("parse", raw.as_bytes(), RequestFormat::Native)
        .expect("the native response normalizes");
    let expected = serde_json::from_str::<Json>(raw).expect("the raw body parses");
    assert_eq!(native.provider_native_response, Some(expected));
    let litellm = CohereParseConfig
        .transform_ocr_response("parse", raw.as_bytes(), RequestFormat::Litellm)
        .expect("the litellm response normalizes");
    assert_eq!(litellm.provider_native_response, None);
}

#[test]
fn the_request_body_must_hold_an_image_document() {
    let image = serde_json::from_str::<Json>(
        r#"{"model":"parse","document":{"type":"image_url","image_url":"https://example.com/a.png"}}"#,
    )
    .expect("the body parses");
    CohereParseConfig
        .validate_request_body(&image)
        .expect("an image body passes");
    let pdf = serde_json::from_str::<Json>(
        r#"{"model":"parse","document":{"type":"document_url","document_url":"https://example.com/a.pdf"}}"#,
    )
    .expect("the body parses");
    assert!(matches!(
        CohereParseConfig.validate_request_body(&pdf),
        Err(OcrError::CohereImageOnly)
    ));
    let missing = serde_json::from_str::<Json>(r#"{"model":"parse"}"#).expect("the body parses");
    assert!(matches!(
        CohereParseConfig.validate_request_body(&missing),
        Err(OcrError::RequestField { path }) if path.starts_with("body.document")
    ));
}

#[test]
fn the_provider_metadata_matches_the_cohere_parse_config() {
    assert_eq!(
        CohereParseConfig.get_supported_ocr_params("parse"),
        &["output_format", "req_format"]
    );
    assert_eq!(
        CohereParseConfig.get_api_key_env_var(),
        Some(COHERE_API_KEY_ENV)
    );
    assert_eq!(CohereParseConfig.secret_names(), vec![COHERE_API_KEY_ENV]);
}
