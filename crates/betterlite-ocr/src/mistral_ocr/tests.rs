#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use rstest::rstest;
use secrecy::SecretString;

use crate::json::{Json, Object};
use crate::ocr_config::connection::{
    OcrConnection, OcrInputSource, OcrSourced, OcrTransportConfig, ResolvedOcrCredentials,
};
use crate::ocr_config::{
    OCR_RESPONSE_MAX_BYTES, OcrProviderConfig as _, OcrTransform, PreparedOcrRequest, Secrets,
};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::{OcrAuthError, OcrError};
use crate::ocr_http::tests::support::{MockServer, ocr_client, reply};
use crate::ocr_http::{CallHooks, OcrWireRequest, ocr};
use crate::ocr_settings::OcrSettings;
use crate::request_format::RequestFormat;
use crate::response::OCRResponse;

use super::{MISTRAL_OCR_API_KEY_ENV_VAR, MistralOcrConfig, build_ocr_url, resolve_headers};

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

fn json_value(text: &str) -> Json {
    serde_json::from_str(text).expect("the test value is JSON")
}

fn params(text: &str) -> Object {
    serde_json::from_str(text).expect("the test options are an object")
}

fn document() -> OcrDocument {
    OcrDocument::DocumentUrl {
        document_url: "https://example.com/a.pdf".to_owned(),
        extra: BTreeMap::new(),
    }
}

fn connection(
    api_base: Option<&str>,
    api_key: Option<&str>,
    extra_headers: Vec<(String, String)>,
    secrets: Secrets,
) -> OcrConnection {
    OcrConnection::new(
        ResolvedOcrCredentials {
            api_key: api_key.map(|key| OcrSourced {
                value: SecretString::from(key),
                source: OcrInputSource::Request,
            }),
            api_base: api_base.map(|base| OcrSourced {
                value: base.to_owned(),
                source: OcrInputSource::Request,
            }),
        },
        OcrTransportConfig {
            extra_headers,
            extra_headers_source: OcrInputSource::Deployment,
            timeout: None,
            max_response_bytes: OCR_RESPONSE_MAX_BYTES,
        },
        OcrSettings::default(),
        secrets,
    )
}

fn request(connection: OcrConnection, optional_params: Object) -> PreparedOcrRequest {
    PreparedOcrRequest {
        model: "mistral-ocr-latest".to_owned(),
        document: document(),
        connection,
        caller_document: false,
        optional_params,
        input_sources: BTreeMap::new(),
        azure_ad_token_provider: None,
    }
}

fn key_secrets(key: &str) -> Secrets {
    Secrets::from([(
        MISTRAL_OCR_API_KEY_ENV_VAR.to_owned(),
        SecretString::from(key),
    )])
}

async fn call(answer: &[u8], options: Object) -> (OCRResponse, String) {
    let server = MockServer::start(vec![reply("200 OK", &[], answer)]).await;
    let prepared = request(
        connection(
            Some(&format!("{}/v1", server.base)),
            Some("sk-test"),
            Vec::new(),
            Secrets::new(),
        ),
        options,
    );
    let client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    let response = ocr(&MistralOcrConfig, &client, &prepared, &NoHooks)
        .await
        .expect("the call succeeds");
    let requests = server.requests().await;
    let sent = requests.first().cloned().expect("one request is sent");
    (response, sent)
}

#[tokio::test]
async fn sends_the_mistral_body_with_the_bearer_key_and_without_control_options() {
    let (_, sent) = call(
        br#"{"pages":[],"model":"served-model"}"#,
        params(r#"{"pages": [0, 2], "extract_header": true, "req_format": "litellm"}"#),
    )
    .await;
    assert!(sent.starts_with("POST /v1/ocr HTTP/1.1"), "{sent}");
    let lowered = sent.to_ascii_lowercase();
    assert!(lowered.contains("authorization: bearer sk-test"), "{sent}");
    assert!(
        sent.contains(
            "\"document\":{\"type\":\"document_url\",\"document_url\":\"https://example.com/a.pdf\"}"
        ),
        "{sent}"
    );
    assert!(sent.contains("\"pages\":[0,2]"), "{sent}");
    assert!(sent.contains("\"extract_header\":true"), "{sent}");
    assert!(!sent.contains("req_format"), "{sent}");
}

#[tokio::test]
async fn decodes_the_response_into_the_shared_ocr_response() {
    let answer = br#"{"pages":[{"index":0,"markdown":"hello","images":[{"id":"img-0","image_base64":"data:image/png;base64,AA=="}],"dimensions":{"width":612,"height":792,"dpi":72},"blocks":[{"type":"title","bbox":{"x":1}}],"confidence_scores":{"mean":0.98}},{"index":1,"markdown":"second","tables":[{"rows":2}],"header":"h","footer":"f"}],"model":"served-model","document_annotation":{"language":"en"},"usage_info":{"pages_processed":2,"credits":1.5,"custom":0},"provider_field":true}"#;
    let (response, _) = call(answer, params("{}")).await;
    assert_eq!(response.model, "served-model");
    assert_eq!(response.object, "ocr");
    assert_eq!(response.pages.len(), 2);
    assert_eq!(response.pages[0].markdown, "hello");
    assert_eq!(
        response.pages[0].extra.get("blocks"),
        Some(&json_value(r#"[{"type": "title", "bbox": {"x": 1}}]"#))
    );
    assert_eq!(
        response.pages[1].extra.get("header"),
        Some(&Json::Text("h".to_owned()))
    );
    assert_eq!(
        response.usage_info.as_ref().and_then(|usage| usage.credits),
        Some(1.5_f64)
    );
    assert_eq!(
        response
            .usage_info
            .as_ref()
            .and_then(|usage| usage.extra.get("custom")),
        Some(&json_value("0"))
    );
    assert_eq!(
        response.document_annotation,
        Some(json_value(r#"{"language": "en"}"#))
    );
    assert_eq!(
        response.extra.get("provider_field"),
        Some(&Json::Bool(true))
    );
}

#[rstest]
#[case::default_base(None, "https://api.mistral.ai/v1/ocr")]
#[case::bare_base(Some("https://example.com"), "https://example.com/v1/ocr")]
#[case::versioned_base(Some("https://example.com/v1"), "https://example.com/v1/ocr")]
#[case::complete_endpoint(Some("https://example.com/v1/ocr"), "https://example.com/v1/ocr")]
#[case::complete_endpoint_with_query(
    Some("https://example.com/v1/ocr?tenant=a"),
    "https://example.com/v1/ocr?tenant=a"
)]
#[case::query_is_kept(
    Some("https://example.com/v1?tenant=a"),
    "https://example.com/v1/ocr?tenant=a"
)]
#[case::base_is_trimmed(Some("  https://example.com/v1  "), "https://example.com/v1/ocr")]
#[case::blank_base_uses_the_default(Some("   "), "https://api.mistral.ai/v1/ocr")]
#[case::trailing_slash(Some("https://example.com/v1/"), "https://example.com/v1/ocr")]
fn completes_a_base_to_one_ocr_url(#[case] api_base: Option<&str>, #[case] expected: &str) {
    assert_eq!(
        build_ocr_url(api_base).expect("the base completes"),
        expected
    );
}

#[rstest]
#[case::not_a_url("not a url")]
#[case::no_path_to_extend("mailto:ops@example.com")]
fn an_invalid_base_fails_with_the_api_base_request_field(#[case] api_base: &str) {
    assert!(matches!(
        build_ocr_url(Some(api_base)),
        Err(OcrError::RequestField { path }) if path == "api_base"
    ));
}

#[test]
fn the_request_key_becomes_the_bearer_header_before_the_environment_key() {
    let explicit = connection(
        None,
        Some("explicit"),
        Vec::new(),
        key_secrets("environment"),
    );
    assert_eq!(
        resolve_headers(&explicit).expect("the request key resolves"),
        [("Authorization".to_owned(), "Bearer explicit".to_owned())]
    );
    let environment = connection(None, None, Vec::new(), key_secrets("environment"));
    assert_eq!(
        resolve_headers(&environment).expect("the environment key resolves"),
        [("Authorization".to_owned(), "Bearer environment".to_owned())]
    );
    let blank_request_key = connection(None, Some(""), Vec::new(), key_secrets("environment"));
    assert_eq!(
        resolve_headers(&blank_request_key).expect("the blank request key falls back"),
        [("Authorization".to_owned(), "Bearer environment".to_owned())]
    );
}

#[rstest]
#[case::lowercase("authorization")]
#[case::mixed_case("Authorization")]
fn a_supplied_authorization_header_wins_over_both_keys(#[case] name: &str) {
    let headers = vec![
        (name.to_owned(), "Bearer forwarded".to_owned()),
        ("X-Trace".to_owned(), "trace-1".to_owned()),
    ];
    let connection = connection(
        None,
        Some("explicit"),
        headers.clone(),
        key_secrets("environment"),
    );
    assert_eq!(
        resolve_headers(&connection).expect("the header wins"),
        headers
    );
}

#[test]
fn extra_headers_follow_the_bearer_header() {
    let headers = vec![("X-Trace".to_owned(), "trace-1".to_owned())];
    let connection = connection(None, Some("explicit"), headers, Secrets::new());
    assert_eq!(
        resolve_headers(&connection).expect("the key resolves"),
        [
            ("Authorization".to_owned(), "Bearer explicit".to_owned()),
            ("X-Trace".to_owned(), "trace-1".to_owned()),
        ]
    );
}

#[rstest]
#[case::absent(None)]
#[case::blank(Some("   "))]
#[case::empty(Some(""))]
fn a_missing_key_is_the_mistral_authentication_error(#[case] api_key: Option<&str>) {
    let environment = key_secrets("   ");
    let connection = connection(None, api_key, Vec::new(), environment);
    let error = resolve_headers(&connection).expect_err("a missing key is an error");
    assert!(matches!(
        error,
        OcrError::Auth(OcrAuthError::MissingApiKey {
            provider: "Mistral",
            environment_variable: MISTRAL_OCR_API_KEY_ENV_VAR,
        })
    ));
}

#[rstest]
#[case::pages("pages", "[0, 1]")]
#[case::include_image_base64("include_image_base64", "true")]
#[case::image_limit("image_limit", "2")]
#[case::image_min_size("image_min_size", "100")]
#[case::bbox_annotation_format("bbox_annotation_format", r#"{"type": "json_schema"}"#)]
#[case::document_annotation_format("document_annotation_format", r#"{"type": "json_schema"}"#)]
#[case::document_annotation_prompt("document_annotation_prompt", r#""extract""#)]
#[case::extract_header("extract_header", "true")]
#[case::extract_footer("extract_footer", "false")]
#[case::table_format("table_format", r#""markdown""#)]
#[case::confidence_scores_granularity("confidence_scores_granularity", r#""word""#)]
#[case::include_blocks("include_blocks", "true")]
#[case::id("id", r#""req-123""#)]
#[case::explicit_null("pages", "null")]
fn a_supported_option_keeps_its_json_value_unchanged(#[case] name: &str, #[case] value: &str) {
    let input = params(&format!(r#"{{"{name}": {value}}}"#));
    let mapped = MistralOcrConfig
        .map_ocr_params(&input, "mistral-ocr-latest")
        .expect("the options map");
    assert_eq!(mapped.get(name), Some(&json_value(value)));
}

#[test]
fn map_ocr_params_drops_unsupported_names_and_leaves_the_input_alone() {
    let input = params(
        r#"{"pages": null, "extract_header": false, "unknown_option": true, "output_format": "markdown"}"#,
    );
    let mapped = MistralOcrConfig
        .map_ocr_params(&input, "mistral-ocr-latest")
        .expect("the options map");
    assert_eq!(
        mapped,
        params(r#"{"pages": null, "extract_header": false}"#)
    );
    assert_eq!(input.get("unknown_option"), Some(&Json::Bool(true)));
    assert_eq!(input.get("pages"), Some(&Json::Null));
}

#[test]
fn transform_ocr_request_places_the_document_as_given_and_the_mapped_options() {
    let options = params(r#"{"extension": {"nested": null}, "pages": [0, 2]}"#);
    let body = MistralOcrConfig
        .transform_ocr_request(
            document(),
            OcrTransform {
                model: "mistral-ocr-latest",
                optional_params: &options,
                headers: &[],
            },
        )
        .expect("the request transforms");
    let value = serde_json::to_string(&body).expect("the body is JSON");
    assert_eq!(
        value,
        r#"{"model":"mistral-ocr-latest","document":{"type":"document_url","document_url":"https://example.com/a.pdf"},"extension":{"nested":null},"pages":[0,2]}"#
    );
}

#[rstest]
#[case::served_model(r#"{"pages": [], "model": "served-model"}"#, "served-model")]
#[case::missing_model(r#"{"pages": []}"#, "mistral-ocr-latest")]
fn a_response_uses_its_own_model_or_the_request_model(
    #[case] payload: &str,
    #[case] expected: &str,
) {
    let response = MistralOcrConfig
        .transform_ocr_response(
            "mistral-ocr-latest",
            payload.as_bytes(),
            RequestFormat::Litellm,
        )
        .expect("the response decodes");
    assert_eq!(response.model, expected);
}

#[test]
fn a_response_without_pages_decodes_to_an_empty_page_list() {
    let response = MistralOcrConfig
        .transform_ocr_response(
            "mistral-ocr-latest",
            br#"{"model":"served-model"}"#,
            RequestFormat::Litellm,
        )
        .expect("the response decodes");
    assert!(response.pages.is_empty());
}

#[rstest]
#[case::non_object_page(r#"{"pages":[42]}"#)]
#[case::missing_markdown(r#"{"pages":[{"index":0}]}"#)]
#[case::non_string_markdown(r#"{"pages":[{"index":0,"markdown":42}]}"#)]
fn an_invalid_page_fails_with_the_page_response_field(
    #[case] payload: &str,
    #[values(RequestFormat::Litellm, RequestFormat::Native)] format: RequestFormat,
) {
    let error = MistralOcrConfig
        .transform_ocr_response("mistral-ocr-latest", payload.as_bytes(), format)
        .expect_err("an invalid page is an error");
    assert!(
        matches!(&error, OcrError::ResponseField { path } if path.starts_with("pages[0]")),
        "{error:?}"
    );
}

#[test]
fn an_explicit_null_model_fails_with_the_model_response_field() {
    let error = MistralOcrConfig
        .transform_ocr_response(
            "mistral-ocr-latest",
            br#"{"pages":[],"model":null}"#,
            RequestFormat::Litellm,
        )
        .expect_err("an explicit null model is an error");
    assert!(matches!(error, OcrError::ResponseField { path } if path == "model"));
}

#[test]
fn a_native_response_format_keeps_the_provider_json_in_the_native_slot() {
    let payload = br#"{"pages":[],"provider_extension":false}"#;
    let native = MistralOcrConfig
        .transform_ocr_response("mistral-ocr-latest", payload, RequestFormat::Native)
        .expect("the native response decodes");
    assert_eq!(
        native.provider_native_response,
        Some(json_value(r#"{"pages": [], "provider_extension": false}"#))
    );
    let litellm = MistralOcrConfig
        .transform_ocr_response("mistral-ocr-latest", payload, RequestFormat::Litellm)
        .expect("the litellm response decodes");
    assert_eq!(litellm.provider_native_response, None);
}

const PAGE_FIELDS_PAYLOAD: &str = r#"{"pages":[{"index":0,"markdown":"hello","images":[{"id":"img-0"}],"dimensions":{"width":612,"height":792,"dpi":72},"blocks":[{"type":"title"}],"confidence_scores":{"mean":0.98},"tables":[{"rows":2}],"hyperlinks":["https://example.com"]}],"usage_info":{"pages_processed":1,"custom":0},"provider_extension":false}"#;

fn decode_page_fields() -> OCRResponse {
    MistralOcrConfig
        .transform_ocr_response(
            "mistral-ocr-latest",
            PAGE_FIELDS_PAYLOAD.as_bytes(),
            RequestFormat::Litellm,
        )
        .expect("the response decodes")
}

#[test]
fn the_response_keeps_page_blocks_confidence_scores_tables_and_hyperlinks() {
    let response = decode_page_fields();
    let page = &response.pages[0];
    assert_eq!(
        page.extra.get("blocks"),
        Some(&json_value(r#"[{"type": "title"}]"#))
    );
    assert_eq!(
        page.extra.get("confidence_scores"),
        Some(&json_value(r#"{"mean": 0.98}"#))
    );
    assert_eq!(
        page.extra.get("tables"),
        Some(&json_value(r#"[{"rows": 2}]"#))
    );
    assert_eq!(
        page.extra.get("hyperlinks"),
        Some(&json_value(r#"["https://example.com"]"#))
    );
    assert_eq!(
        page.dimensions
            .as_ref()
            .and_then(|dimensions| dimensions.width),
        Some(612)
    );
}

#[test]
fn the_response_keeps_usage_and_unknown_root_fields() {
    let response = decode_page_fields();
    assert_eq!(
        response
            .usage_info
            .as_ref()
            .and_then(|usage| usage.pages_processed),
        Some(1)
    );
    assert_eq!(
        response
            .usage_info
            .as_ref()
            .and_then(|usage| usage.extra.get("custom")),
        Some(&json_value("0"))
    );
    assert_eq!(
        response.extra.get("provider_extension"),
        Some(&Json::Bool(false))
    );
}

#[test]
fn exposes_the_api_key_environment_supported_options_and_secret_names() {
    assert_eq!(
        MistralOcrConfig.get_api_key_env_var(),
        Some(MISTRAL_OCR_API_KEY_ENV_VAR)
    );
    assert_eq!(
        MistralOcrConfig.get_supported_ocr_params("mistral-ocr-latest"),
        [
            "pages",
            "include_image_base64",
            "image_limit",
            "image_min_size",
            "bbox_annotation_format",
            "document_annotation_format",
            "document_annotation_prompt",
            "extract_header",
            "extract_footer",
            "table_format",
            "confidence_scores_granularity",
            "include_blocks",
            "id",
        ]
    );
    assert_eq!(
        MistralOcrConfig.secret_names(),
        [
            "MISTRAL_API_KEY",
            "MISTRAL_AZURE_API_KEY",
            "MISTRAL_AZURE_API_BASE",
        ]
    );
}
