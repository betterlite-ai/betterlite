#![cfg(test)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use rstest::rstest;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use serde_json::Number;

use crate::json::{Json, Object};
use crate::mistral_ocr::MistralOcrConfig;
use crate::ocr_config::OCR_RESPONSE_MAX_BYTES;
use crate::ocr_config::OcrProviderConfig as _;
use crate::ocr_config::connection::OcrInputSource;
use crate::ocr_config::decode::compose_body;
use crate::ocr_document::OcrDocument;
use crate::ocr_error::OcrError;
use crate::ocr_input::OcrDocumentInput;
use crate::ocr_input::arguments::{
    consumed_optional_param_names, consumed_optional_params, is_secret_param, is_supported_request,
};
use crate::ocr_input::wire::{
    OcrWireRequest, decode_document, decode_request, decode_request_input,
};
use crate::ocr_provider::OcrConfigKind;

fn object(entries: &[(&str, Json)]) -> Object {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: u64) -> Json {
    Json::Number(Number::from(value))
}

fn document(source: &str) -> Json {
    Json::Object(object(&[
        ("type", text("document_url")),
        ("document_url", text(source)),
    ]))
}

fn wire(model: &str, document: Json) -> OcrWireRequest {
    OcrWireRequest {
        model: model.to_owned(),
        document,
        api_key: None,
        api_base: None,
        custom_llm_provider: None,
        extra_headers: None,
        optional_params: Object::new(),
        input_sources: BTreeMap::new(),
        timeout_seconds: None,
    }
}

fn request_with(document: Json) -> OcrWireRequest {
    wire("mistral-ocr-latest", document)
}

#[test]
fn document_inputs_convert_from_documents_and_paths() {
    let document = OcrDocument::DocumentUrl {
        document_url: "data:application/pdf;base64,YWJj".to_owned(),
        extra: BTreeMap::new(),
    };
    assert_eq!(
        OcrDocumentInput::from(document.clone()),
        OcrDocumentInput::Document(document)
    );
    assert_eq!(
        OcrDocumentInput::from(PathBuf::from("/tmp/scan.pdf")),
        OcrDocumentInput::Path {
            path: PathBuf::from("/tmp/scan.pdf"),
            mime_type: None,
        }
    );
}

#[test]
fn wire_request_decodes_every_field() {
    let request = decode_request(OcrWireRequest {
        model: "mistral/model-name".to_owned(),
        document: document("data:application/pdf;base64,YWJj"),
        api_key: Some(SecretString::from("sk-test")),
        api_base: Some("https://api.example.com".to_owned()),
        custom_llm_provider: None,
        extra_headers: Some(object(&[("x-trace", text("1"))])),
        optional_params: object(&[("pages", Json::List(vec![number(0), number(1)]))]),
        input_sources: BTreeMap::from([("api_key".to_owned(), "request".to_owned())]),
        timeout_seconds: Some(2.5_f64),
    })
    .expect("the wire request decodes");

    assert_eq!(request.model, "model-name");
    assert_eq!(
        request.document,
        OcrDocumentInput::Document(OcrDocument::DocumentUrl {
            document_url: "data:application/pdf;base64,YWJj".to_owned(),
            extra: BTreeMap::new(),
        })
    );
    let api_key = request.credentials.api_key.expect("the key is kept");
    assert_eq!(api_key.value.expose_secret(), "sk-test");
    assert_eq!(api_key.source, OcrInputSource::Request);
    let api_base = request.credentials.api_base.expect("the base is kept");
    assert_eq!(api_base.value, "https://api.example.com");
    assert_eq!(api_base.source, OcrInputSource::Deployment);
    assert_eq!(
        request.transport.extra_headers,
        vec![("x-trace".to_owned(), "1".to_owned())]
    );
    assert_eq!(
        request.transport.extra_headers_source,
        OcrInputSource::Deployment
    );
    assert_eq!(request.transport.timeout, Some(Duration::from_millis(2500)));
    assert_eq!(request.config, OcrConfigKind::Mistral);
    assert_eq!(request.optional_params.len(), 1);
    assert!(request.optional_params.contains_key("pages"));
    assert_eq!(request.input_sources.len(), 1);
}

#[rstest]
#[case::negative(-1.0_f64)]
#[case::not_a_number(f64::NAN)]
#[case::infinite(f64::INFINITY)]
fn bad_timeout_fails_with_the_field(#[case] timeout: f64) {
    let Err(error) = decode_request(OcrWireRequest {
        timeout_seconds: Some(timeout),
        ..request_with(document("https://example.com/a.pdf"))
    }) else {
        panic!("the timeout is rejected");
    };
    assert!(
        matches!(&error, OcrError::RequestField { path } if path == "timeout_seconds"),
        "{error}"
    );
}

#[test]
fn non_string_header_value_fails_with_its_path() {
    let Err(error) = decode_request(OcrWireRequest {
        extra_headers: Some(object(&[("x-a", number(1))])),
        ..request_with(document("https://example.com/a.pdf"))
    }) else {
        panic!("the header is rejected");
    };
    assert!(
        matches!(&error, OcrError::RequestField { path } if path == "extra_headers.x-a"),
        "{error}"
    );
}

#[rstest]
#[case::document_without_url(Json::Object(object(&[("type", text("document_url"))])))]
#[case::image_without_url(Json::Object(object(&[("type", text("image_url"))])))]
fn url_less_documents_need_their_url(#[case] document: Json) {
    let error = decode_document(&document).expect_err("the document is rejected");
    assert!(matches!(error, OcrError::MissingDocumentUrl), "{error}");
}

#[test]
fn a_document_url_of_null_is_a_bad_field_not_a_missing_url() {
    let error = decode_document(&Json::Object(object(&[
        ("type", text("document_url")),
        ("document_url", Json::Null),
    ])))
    .expect_err("the document is rejected");
    assert!(
        matches!(&error, OcrError::RequestField { path } if path.starts_with("document")),
        "{error}"
    );
}

#[rstest]
#[case::not_an_object(Json::List(Vec::new()))]
#[case::missing_type(Json::Object(object(&[("document_url", text("https://example.com/a.pdf"))])))]
#[case::unsupported_type(Json::Object(object(&[("type", text("text"))])))]
#[case::type_not_a_string(Json::Object(object(&[("type", number(1))])))]
fn malformed_documents_are_request_fields(#[case] document: Json) {
    let error = decode_document(&document).expect_err("the document is rejected");
    assert!(matches!(error, OcrError::RequestField { .. }), "{error}");
    assert_eq!(error.http_status_code(), Some(400));
}

#[test]
fn a_file_document_never_reaches_the_wire_decoder() {
    let error = decode_document(&Json::Object(object(&[
        ("type", text("file")),
        ("file", text("/etc/passwd")),
    ])))
    .expect_err("a file document is rejected");
    assert!(matches!(error, OcrError::RequestField { .. }), "{error}");
}

#[rstest]
#[case::absent(None, Some(OCR_RESPONSE_MAX_BYTES))]
#[case::valid(Some(number(1024)), Some(1024))]
#[case::zero(Some(number(0)), None)]
#[case::over_the_cap(Some(number(u64::try_from(OCR_RESPONSE_MAX_BYTES).expect("fits").checked_add(1).expect("fits"))), None)]
#[case::a_string(Some(text("1024")), None)]
#[case::null(Some(Json::Null), None)]
#[case::fractional(Some(Json::Number(Number::from_f64(1024.5).expect("finite"))), None)]
fn max_response_bytes_is_read_and_removed_from_the_options(
    #[case] value: Option<Json>,
    #[case] expected: Option<usize>,
) {
    let mut optional_params = Object::new();
    if let Some(limit) = value {
        let _previous = optional_params.insert("max_response_bytes".to_owned(), limit);
    }
    let decoded = decode_request(OcrWireRequest {
        optional_params,
        ..request_with(document("https://example.com/a.pdf"))
    });
    let Some(limit) = expected else {
        let Err(error) = decoded else {
            panic!("the limit is rejected");
        };
        assert!(
            matches!(&error, OcrError::RequestField { path } if path == "max_response_bytes"),
            "{error}"
        );
        return;
    };
    let request = decoded.expect("the limit is accepted");
    assert_eq!(request.transport.max_response_bytes, limit);
    assert!(!request.optional_params.contains_key("max_response_bytes"));
}

#[test]
fn an_unknown_input_source_is_a_request_field() {
    let Err(error) = decode_request(OcrWireRequest {
        input_sources: BTreeMap::from([("api_key".to_owned(), "somewhere".to_owned())]),
        ..request_with(document("https://example.com/a.pdf"))
    }) else {
        panic!("the source is rejected");
    };
    assert!(
        matches!(&error, OcrError::RequestField { path } if path == "input_sources.api_key"),
        "{error}"
    );
}

#[rstest]
#[case::request("request", OcrInputSource::Request)]
#[case::deployment("deployment", OcrInputSource::Deployment)]
#[case::environment("environment", OcrInputSource::Environment)]
fn input_sources_decode_to_their_kind(#[case] name: &str, #[case] expected: OcrInputSource) {
    let request = decode_request(OcrWireRequest {
        api_key: Some(SecretString::from("sk-test")),
        input_sources: BTreeMap::from([("api_key".to_owned(), name.to_owned())]),
        ..request_with(document("https://example.com/a.pdf"))
    })
    .expect("the request decodes");
    let api_key = request.credentials.api_key.expect("the key is kept");
    assert_eq!(api_key.source, expected);
}

#[test]
fn decode_request_input_keeps_a_path_document() {
    let request = decode_request_input(OcrWireRequest::<OcrDocumentInput> {
        model: "mistral/model".to_owned(),
        document: OcrDocumentInput::Path {
            path: PathBuf::from("/tmp/scan.pdf"),
            mime_type: None,
        },
        api_key: None,
        api_base: None,
        custom_llm_provider: None,
        extra_headers: None,
        optional_params: Object::new(),
        input_sources: BTreeMap::new(),
        timeout_seconds: None,
    })
    .expect("the request decodes");
    assert!(
        matches!(&request.document, OcrDocumentInput::Path { path, .. } if path == &PathBuf::from("/tmp/scan.pdf"))
    );
}

#[rstest]
#[case::custom_provider_wins("mistral/x", Some("cohere"), "mistral/x", OcrConfigKind::Cohere)]
#[case::model_prefix("vertex_ai/mistral-ocr", None, "mistral-ocr", OcrConfigKind::VertexAi)]
#[case::no_provider_defaults_to_mistral(
    "mistral-ocr-latest",
    None,
    "mistral-ocr-latest",
    OcrConfigKind::Mistral
)]
#[case::deepseek_prefix(
    "vertex_ai/deepseek-ocr",
    None,
    "deepseek-ocr",
    OcrConfigKind::VertexDeepSeek
)]
fn provider_resolution_uses_the_custom_provider_then_the_model_prefix(
    #[case] model: &str,
    #[case] custom_llm_provider: Option<&str>,
    #[case] expected_model: &str,
    #[case] expected_kind: OcrConfigKind,
) {
    let request = decode_request(OcrWireRequest {
        custom_llm_provider: custom_llm_provider.map(str::to_owned),
        ..wire(model, document("https://example.com/a.pdf"))
    })
    .expect("the request decodes");
    assert_eq!(request.model, expected_model);
    assert_eq!(request.config, expected_kind);
}

#[rstest]
#[case::mistral("mistral-ocr-latest", None, true)]
#[case::provider_only("model", Some("mistral"), true)]
#[case::azure_ai("pixtral-12b", Some("azure_ai"), true)]
#[case::reducto("parse-v3", Some("reducto"), true)]
#[case::vertex_deepseek("deepseek-ocr", Some("vertex_ai"), true)]
#[case::unsupported("model", Some("anthropic"), false)]
#[case::unsupported_prefix("openai/gpt-4o", None, false)]
fn supported_requests_match_provider_resolution(
    #[case] model: &str,
    #[case] provider: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(is_supported_request(model, provider), expected);
}

#[rstest]
#[case::mistral("mistral-ocr-latest", None, "pages", "vertex_project")]
#[case::cohere("parse", Some("cohere"), "output_format", "pages")]
#[case::azure_ai("model", Some("azure_ai"), "pages", "vertex_project")]
#[case::textract(
    "detect-document-text",
    Some("aws_textract"),
    "aws_secret_access_key",
    "vertex_project"
)]
#[case::vertex("mistral-ocr", Some("vertex_ai"), "pages", "azure_ad_token")]
fn consumed_names_add_the_provider_and_auth_options(
    #[case] model: &str,
    #[case] provider: Option<&str>,
    #[case] included: &str,
    #[case] excluded: &str,
) {
    let names = consumed_optional_param_names(model, provider).expect("the provider resolves");
    assert!(names.contains(&"req_format"), "req_format is consumed");
    assert!(names.contains(&"extra_body"), "extra_body is consumed");
    assert!(
        names.contains(&"max_response_bytes"),
        "max_response_bytes is consumed"
    );
    assert!(names.contains(&included), "{included} is consumed");
    assert!(!names.contains(&excluded), "{excluded} is not consumed");
}

#[test]
fn consumed_names_add_the_aws_credentials_last() {
    let names = consumed_optional_param_names("detect-document-text", Some("aws_textract"))
        .expect("textract resolves");
    assert_eq!(
        names,
        [
            "req_format",
            "extra_body",
            "max_response_bytes",
            "aws_access_key_id",
            "aws_secret_access_key",
            "aws_session_token",
            "aws_region_name",
            "aws_session_name",
            "aws_profile_name",
            "aws_role_name",
            "aws_web_identity_token",
            "aws_sts_endpoint",
            "aws_external_id",
        ]
    );
}

#[test]
fn consumed_names_add_the_azure_credentials_last() {
    let names = consumed_optional_param_names("model", Some("azure_ai")).expect("azure resolves");
    assert_eq!(
        names,
        [
            "req_format",
            "extra_body",
            "max_response_bytes",
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
            "azure_ad_token",
            "tenant_id",
            "client_id",
            "client_secret",
            "azure_scope",
            "azure_authority_host",
            "azure_credential",
            "azure_federated_token_file",
            "enable_azure_ad_token_refresh",
        ]
    );
}

#[test]
fn consumed_names_add_the_vertex_credentials_last() {
    let names =
        consumed_optional_param_names("mistral-ocr", Some("vertex_ai")).expect("vertex resolves");
    assert_eq!(
        names,
        [
            "req_format",
            "extra_body",
            "max_response_bytes",
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
            "vertex_credentials",
            "vertex_ai_credentials",
            "vertex_project",
            "vertex_ai_project",
            "vertex_location",
            "vertex_ai_location",
        ]
    );
}

#[test]
fn vertex_deepseek_consumes_the_chat_sampling_options() {
    let names = consumed_optional_param_names("deepseek-ocr", Some("vertex_ai"))
        .expect("deepseek resolves");
    assert_eq!(
        names,
        [
            "req_format",
            "extra_body",
            "max_response_bytes",
            "vertex_credentials",
            "vertex_ai_credentials",
            "vertex_project",
            "vertex_ai_project",
            "vertex_location",
            "vertex_ai_location",
            "stream",
            "temperature",
            "max_tokens",
            "top_p",
            "n",
            "stop",
        ]
    );
    let page_names =
        consumed_optional_param_names("mistral-ocr", Some("vertex_ai")).expect("vertex resolves");
    assert!(!page_names.contains(&"temperature"));
}

#[test]
fn consumed_params_mark_only_the_credentials_as_secret() {
    let azure = consumed_optional_params("model", Some("azure_ai")).expect("azure resolves");
    assert!(
        azure
            .iter()
            .any(|spec| spec.name == "client_secret" && spec.secret)
    );
    assert!(
        azure
            .iter()
            .any(|spec| spec.name == "tenant_id" && !spec.secret)
    );
    let vertex =
        consumed_optional_params("deepseek-ocr", Some("vertex_ai")).expect("deepseek resolves");
    assert!(
        vertex
            .iter()
            .any(|spec| spec.name == "vertex_credentials" && spec.secret)
    );
    assert!(
        vertex
            .iter()
            .any(|spec| spec.name == "vertex_project" && !spec.secret)
    );
    assert!(
        vertex
            .iter()
            .any(|spec| spec.name == "temperature" && !spec.secret)
    );
}

#[rstest]
#[case::azure_ad_token("azure_ad_token", true)]
#[case::client_secret("client_secret", true)]
#[case::federated_file("azure_federated_token_file", true)]
#[case::vertex_credentials("vertex_credentials", true)]
#[case::vertex_ai_credentials("vertex_ai_credentials", true)]
#[case::aws_secret_key("aws_secret_access_key", true)]
#[case::aws_session_token("aws_session_token", true)]
#[case::aws_web_identity("aws_web_identity_token", true)]
#[case::pages("pages", false)]
#[case::tenant("tenant_id", false)]
fn only_credential_options_are_secret(#[case] name: &str, #[case] expected: bool) {
    assert_eq!(is_secret_param(name), expected);
}

#[test]
fn consumed_options_stay_out_of_the_provider_mapper() {
    let params = object(&[
        ("pages", Json::List(vec![number(0)])),
        ("vertex_project", text("project")),
        ("req_format", text("litellm")),
        ("future_ocr_option", Json::Bool(true)),
    ]);
    let mapped = MistralOcrConfig
        .map_ocr_params(&params, "mistral-ocr-latest")
        .expect("the params map");
    assert_eq!(mapped, object(&[("pages", Json::List(vec![number(0)]))]));
    assert!(!mapped.contains_key("future_ocr_option"));
}

#[test]
fn an_unknown_option_is_left_for_the_body() {
    let json = Object::from([
        ("model".to_owned(), text("mistral-ocr-latest")),
        ("future_ocr_option".to_owned(), Json::Bool(true)),
        ("pages".to_owned(), Json::List(vec![number(0)])),
        ("vertex_project".to_owned(), text("project")),
        ("req_format".to_owned(), text("litellm")),
    ]);
    let composed =
        compose_body(&json, &Json::Object(Object::new()), &["pages"]).expect("the body composes");
    let Json::Object(fields) = composed else {
        panic!("the body is an object");
    };
    assert_eq!(fields.get("future_ocr_option"), Some(&Json::Bool(true)));
    assert!(!fields.contains_key("pages"));
    assert!(!fields.contains_key("vertex_project"));
    assert!(!fields.contains_key("req_format"));
}
