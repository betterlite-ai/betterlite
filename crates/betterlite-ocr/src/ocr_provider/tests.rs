#![cfg(test)]

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::cohere_ocr::COHERE_HEALTH_CHECK_DATA_URI;
use crate::json::{Json, Object};
use crate::ocr_config::PreparedOcrRequest;
use crate::ocr_config::connection::{OcrCredentialInputs, OcrInputSource, OcrSourced};
use crate::ocr_document::{InlineDocument, OcrDocument};
use crate::ocr_error::{OcrError, OcrSecretError};
use crate::ocr_http::tests::support::{MockServer, object, ocr_client, prepared_request, reply};
use crate::ocr_http::{OcrClient, OcrWireRequest, SecretSource};
use crate::ocr_provider::handler::{OcrCallContext, OcrInterceptors, perform_ocr_request};
use crate::ocr_provider::resolve::resolve_provider_config;
use crate::ocr_provider::{
    OcrConfigKind, azure_ocr_route, get_api_key_env_var, get_health_check_document,
    passthrough_response,
};
use crate::ocr_settings::OcrSettings;

const MISTRAL_OCR_PARAMS: &[&str] = &[
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
];
const COHERE_PARSE_PARAMS: &[&str] = &["output_format", "req_format"];
const DOC_INTELLIGENCE_PARAMS: &[&str] = &["pages", "features", "req_format"];
const REDUCTO_V3_PARAMS: &[&str] = &["formatting", "retrieval", "settings"];
const REDUCTO_LEGACY_PARAMS: &[&str] = &["enhance"];
const TEXTRACT_ANALYZE_PARAMS: &[&str] = &["feature_types"];
const NO_PARAMS: &[&str] = &[];
const MISTRAL_PASSTHROUGH_BODY: &[u8] = br#"{
    "pages": [{"index": 0, "markdown": "page one"}, {"index": 1, "markdown": "page two"}],
    "model": "mistral-document-ai-2512",
    "usage_info": {"pages_processed": 2}
}"#;
const COHERE_PASSTHROUGH_BODY: &[u8] =
    br#"{"id": "parse-1", "pages": [], "meta": {"billed_units": {"pages": 3}}}"#;
const DOCUMENT_INTELLIGENCE_PASSTHROUGH_BODY: &[u8] = br#"{
    "status": "succeeded",
    "analyzeResult": {
        "content": "line one",
        "pages": [{"pageNumber": 1, "unit": "inch", "width": 8.5, "height": 11, "lines": [{"content": "line one"}]}]
    }
}"#;
const MISTRAL_WIRE_BODY: &[u8] =
    br#"{"pages":[{"index":0,"markdown":"hello"}],"model":"served-model"}"#;
const SECRET_OPTION_NAMES: [&str; 8] = [
    "azure_ad_token",
    "client_secret",
    "azure_federated_token_file",
    "vertex_credentials",
    "vertex_ai_credentials",
    "aws_secret_access_key",
    "aws_session_token",
    "aws_web_identity_token",
];

fn params(text: &str) -> Object {
    serde_json::from_str(text).expect("the test options are an object")
}

fn mistral_request(base: &str) -> PreparedOcrRequest {
    let mut request = prepared_request(base.to_owned(), Object::new());
    request.model = "mistral-ocr-latest".to_owned();
    request.connection.api_key = Some(SecretString::from("sk-test"));
    request
}

fn context_params() -> Object {
    params(
        r#"{
            "pages": [0, 2],
            "azure_ad_token": "ad",
            "client_secret": "shh",
            "azure_federated_token_file": "/token",
            "vertex_credentials": "{}",
            "vertex_ai_credentials": "{}",
            "aws_secret_access_key": "secret",
            "aws_session_token": "session",
            "aws_web_identity_token": "identity",
            "req_format": "litellm",
            "future": true
        }"#,
    )
}

fn test_client() -> OcrClient {
    ocr_client(OcrSettings::default(), Duration::from_secs(5))
}

struct FixedSecrets(BTreeMap<String, SecretString>);

impl SecretSource for FixedSecrets {
    fn secret(&self, name: &str) -> Result<Option<SecretString>, OcrSecretError> {
        Ok(self.0.get(name).cloned())
    }
}

struct FailingSecrets;

impl SecretSource for FailingSecrets {
    fn secret(&self, name: &str) -> Result<Option<SecretString>, OcrSecretError> {
        Err(OcrSecretError::Resolution {
            name: name.to_owned(),
            detail: "the secret store is unreachable".to_owned(),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
struct SeenContext {
    model: String,
    provider: &'static str,
    optional_params: Object,
    secret_fields: Vec<String>,
    api_key: Option<String>,
}

#[derive(Default)]
struct RecordingInterceptors {
    bodies: Mutex<Vec<Vec<u8>>>,
    contexts: Mutex<Vec<SeenContext>>,
}

impl RecordingInterceptors {
    fn response_bodies(&self) -> Vec<Vec<u8>> {
        self.bodies.lock().expect("the record locks").clone()
    }

    fn call_contexts(&self) -> Vec<SeenContext> {
        self.contexts.lock().expect("the record locks").clone()
    }
}

#[async_trait]
impl OcrInterceptors for RecordingInterceptors {
    async fn before_provider_request(
        &self,
        mut request: OcrWireRequest,
        context: &OcrCallContext,
    ) -> Result<OcrWireRequest, OcrError> {
        self.contexts
            .lock()
            .expect("the record locks")
            .push(SeenContext {
                model: context.model.clone(),
                provider: context.custom_llm_provider,
                optional_params: context.optional_params.clone(),
                secret_fields: context.secret_fields.clone(),
                api_key: context
                    .api_key
                    .as_ref()
                    .map(|key| key.expose_secret().to_owned()),
            });
        let Json::Object(body) = &mut request.body else {
            return Err(OcrError::InvalidRequest(
                "the interceptor expects an object body".to_owned(),
            ));
        };
        let _previous = body.insert("extract_header".to_owned(), Json::Bool(true));
        request
            .headers
            .push(("x-hook".to_owned(), "seen".to_owned()));
        Ok(request)
    }

    async fn response_received(&self, body: &[u8]) -> Result<(), OcrError> {
        self.bodies
            .lock()
            .expect("the record locks")
            .push(body.to_vec());
        Ok(())
    }
}

#[rstest]
#[case::mistral_prefix(
    "mistral/mistral-ocr-latest",
    None,
    "mistral-ocr-latest",
    OcrConfigKind::Mistral
)]
#[case::mistral_explicit(
    "mistral-ocr-latest",
    Some("mistral"),
    "mistral-ocr-latest",
    OcrConfigKind::Mistral
)]
#[case::matching_prefix_is_stripped(
    "mistral/mistral-ocr-latest",
    Some("mistral"),
    "mistral-ocr-latest",
    OcrConfigKind::Mistral
)]
#[case::foreign_prefix_is_kept(
    "mistral/mistral-ocr-latest",
    Some("vertex_ai"),
    "mistral/mistral-ocr-latest",
    OcrConfigKind::VertexAi
)]
#[case::empty_explicit_uses_the_prefix("mistral/ocr", Some(""), "ocr", OcrConfigKind::Mistral)]
#[case::textract_detect(
    "aws_textract/detect-document-text",
    None,
    "detect-document-text",
    OcrConfigKind::AwsTextract
)]
#[case::textract_analyze(
    "aws_textract/analyze-document",
    None,
    "analyze-document",
    OcrConfigKind::AwsTextractAnalyze
)]
#[case::textract_analyze_case(
    "aws_textract/Analyze-Document",
    None,
    "Analyze-Document",
    OcrConfigKind::AwsTextractAnalyze
)]
#[case::cohere("cohere/parse", None, "parse", OcrConfigKind::Cohere)]
#[case::azure_mistral(
    "azure_ai/mistral-document-ai-2512",
    None,
    "mistral-document-ai-2512",
    OcrConfigKind::AzureAi
)]
#[case::azure_parse_only("azure_ai/parse-v5", None, "parse-v5", OcrConfigKind::AzureAi)]
#[case::azure_invoice(
    "azure_ai/invoice-parser",
    None,
    "invoice-parser",
    OcrConfigKind::AzureAi
)]
#[case::azure_cohere(
    "azure_ai/Cohere-parse-v5",
    None,
    "Cohere-parse-v5",
    OcrConfigKind::AzureCohere
)]
#[case::azure_cohere_slashed(
    "azure_ai/cohere/parse-v5",
    None,
    "cohere/parse-v5",
    OcrConfigKind::AzureCohere
)]
#[case::azure_cohere_without_parse(
    "azure_ai/cohere-embed-v3",
    None,
    "cohere-embed-v3",
    OcrConfigKind::AzureAi
)]
#[case::azure_doc_intelligence(
    "azure_ai/doc-intelligence/prebuilt-layout",
    None,
    "doc-intelligence/prebuilt-layout",
    OcrConfigKind::AzureDocumentIntelligence
)]
#[case::azure_doc_intelligence_joined(
    "azure_ai/documentintelligence/prebuilt-layout",
    None,
    "documentintelligence/prebuilt-layout",
    OcrConfigKind::AzureDocumentIntelligence
)]
#[case::azure_doc_intelligence_case(
    "azure_ai/Doc-Intelligence/prebuilt-layout",
    None,
    "Doc-Intelligence/prebuilt-layout",
    OcrConfigKind::AzureDocumentIntelligence
)]
#[case::azure_doc_intelligence_wins(
    "azure_ai/doc-intelligence/cohere-parse",
    None,
    "doc-intelligence/cohere-parse",
    OcrConfigKind::AzureDocumentIntelligence
)]
#[case::reducto_legacy(
    "reducto/parse-legacy",
    None,
    "parse-legacy",
    OcrConfigKind::ReductoLegacy
)]
#[case::reducto_legacy_mixed_case(
    "reducto/Parse-Legacy",
    None,
    "Parse-Legacy",
    OcrConfigKind::ReductoLegacy
)]
#[case::reducto_legacy_suffix(
    "reducto/parse-legacy-v2",
    None,
    "parse-legacy-v2",
    OcrConfigKind::ReductoV3
)]
#[case::reducto_v3("reducto/parse-v3", None, "parse-v3", OcrConfigKind::ReductoV3)]
#[case::vertex_mistral("vertex_ai/mistral-ocr", None, "mistral-ocr", OcrConfigKind::VertexAi)]
#[case::vertex_deepseek(
    "vertex_ai/deepseek-ocr",
    None,
    "deepseek-ocr",
    OcrConfigKind::VertexDeepSeek
)]
#[case::vertex_deepseek_case(
    "vertex_ai/DeepSeek-OCR",
    None,
    "DeepSeek-OCR",
    OcrConfigKind::VertexDeepSeek
)]
fn resolve_provider_models_select_their_kind(
    #[case] model: &str,
    #[case] provider: Option<&str>,
    #[case] expected_model: &str,
    #[case] expected_kind: OcrConfigKind,
) {
    let (resolved, kind) = resolve_provider_config(model, provider).expect("the provider resolves");
    assert_eq!(resolved, expected_model);
    assert_eq!(kind, expected_kind);
}

#[rstest]
#[case::bare_model("mistral-ocr-latest")]
#[case::empty_prefix("/model")]
#[case::empty_model("provider/")]
fn resolve_without_a_known_provider_falls_back_to_mistral(#[case] model: &str) {
    let (resolved, kind) = resolve_provider_config(model, None).expect("the fallback resolves");
    assert_eq!(resolved, model);
    assert_eq!(kind, OcrConfigKind::Mistral);
}

#[test]
fn resolve_treats_an_empty_explicit_provider_as_missing() {
    let (resolved, kind) =
        resolve_provider_config("mistral-ocr-latest", Some("")).expect("the fallback resolves");
    assert_eq!(resolved, "mistral-ocr-latest");
    assert_eq!(kind, OcrConfigKind::Mistral);
}

#[rstest]
#[case::anthropic_prefix("anthropic/claude", None, "anthropic")]
#[case::bedrock_prefix("bedrock/anthropic.claude", None, "bedrock")]
#[case::openai_prefix("openai/gpt-4o", None, "openai")]
#[case::openai_like_prefix("openai_like/llama", None, "openai_like")]
#[case::explicit_anthropic("model", Some("anthropic"), "anthropic")]
#[case::explicit_openai("model", Some("openai"), "openai")]
#[case::unknown_provider("model", Some("not_a_provider"), "not_a_provider")]
#[case::provider_name_is_case_sensitive("Mistral/ocr", None, "Mistral")]
fn resolve_rejects_providers_that_are_not_ocr(
    #[case] model: &str,
    #[case] provider: Option<&str>,
    #[case] expected: &str,
) {
    let error = resolve_provider_config(model, provider).expect_err("the provider is refused");
    assert!(matches!(&error, OcrError::InvalidProvider(name) if name == expected));
    assert_eq!(error.http_status_code(), Some(400));
}

#[rstest]
#[case::misspelled_operation("aws_textract/analyse-document", "analyse-document")]
#[case::operation_name_from_the_api("aws_textract/AnalyzeDocument", "AnalyzeDocument")]
fn resolve_refuses_textract_models_outside_its_operations(
    #[case] model: &str,
    #[case] refused: &str,
) {
    let error = resolve_provider_config(model, None).expect_err("the operation is refused");
    assert!(matches!(
        &error,
        OcrError::InvalidModel { provider, model: refused_model, .. }
            if *provider == "aws_textract" && refused_model.as_str() == refused
    ));
}

#[rstest]
#[case::aws_textract("aws_textract/detect-document-text", NO_PARAMS)]
#[case::aws_textract_analyze("aws_textract/analyze-document", TEXTRACT_ANALYZE_PARAMS)]
#[case::cohere("cohere/parse", COHERE_PARSE_PARAMS)]
#[case::mistral("mistral/ocr", MISTRAL_OCR_PARAMS)]
#[case::azure_ai("azure_ai/ocr", MISTRAL_OCR_PARAMS)]
#[case::azure_cohere("azure_ai/cohere-parse", COHERE_PARSE_PARAMS)]
#[case::azure_document_intelligence(
    "azure_ai/doc-intelligence/prebuilt-layout",
    DOC_INTELLIGENCE_PARAMS
)]
#[case::reducto_legacy("reducto/parse-legacy", REDUCTO_LEGACY_PARAMS)]
#[case::reducto_v3("reducto/parse-v3", REDUCTO_V3_PARAMS)]
#[case::vertex_ai("vertex_ai/mistral-ocr", MISTRAL_OCR_PARAMS)]
#[case::vertex_deepseek("vertex_ai/deepseek-ocr", NO_PARAMS)]
fn forward_supported_ocr_params_follow_the_selected_config(
    #[case] model: &str,
    #[case] expected: &[&str],
) {
    let (resolved, kind) = resolve_provider_config(model, None).expect("the provider resolves");
    assert_eq!(kind.get_supported_ocr_params(&resolved), expected);
}

#[rstest]
#[case::aws_textract(OcrConfigKind::AwsTextract, "aws_textract")]
#[case::aws_textract_analyze(OcrConfigKind::AwsTextractAnalyze, "aws_textract")]
#[case::cohere(OcrConfigKind::Cohere, "cohere")]
#[case::mistral(OcrConfigKind::Mistral, "mistral")]
#[case::azure_ai(OcrConfigKind::AzureAi, "azure_ai")]
#[case::azure_cohere(OcrConfigKind::AzureCohere, "azure_ai")]
#[case::azure_document_intelligence(OcrConfigKind::AzureDocumentIntelligence, "azure_ai")]
#[case::reducto_legacy(OcrConfigKind::ReductoLegacy, "reducto")]
#[case::reducto_v3(OcrConfigKind::ReductoV3, "reducto")]
#[case::vertex_ai(OcrConfigKind::VertexAi, "vertex_ai")]
#[case::vertex_deepseek(OcrConfigKind::VertexDeepSeek, "vertex_ai")]
fn forward_provider_names_match_the_litellm_names(#[case] kind: OcrConfigKind, #[case] name: &str) {
    assert_eq!(kind.provider(), name);
}

#[rstest]
#[case(OcrConfigKind::AwsTextract)]
#[case(OcrConfigKind::AwsTextractAnalyze)]
#[case(OcrConfigKind::Cohere)]
#[case(OcrConfigKind::Mistral)]
#[case(OcrConfigKind::AzureAi)]
#[case(OcrConfigKind::AzureCohere)]
#[case(OcrConfigKind::AzureDocumentIntelligence)]
#[case(OcrConfigKind::ReductoLegacy)]
#[case(OcrConfigKind::ReductoV3)]
#[case(OcrConfigKind::VertexAi)]
#[case(OcrConfigKind::VertexDeepSeek)]
fn forward_secret_names_carry_the_api_key_without_duplicates(#[case] kind: OcrConfigKind) {
    let names = kind.secret_names();
    let unique: HashSet<&str> = names.iter().copied().collect();
    assert_eq!(names.len(), unique.len());
    if let Some(api_key) = kind.get_api_key_env_var() {
        assert!(names.contains(&api_key));
    }
}

#[rstest]
#[case::mistral(OcrConfigKind::Mistral, "MISTRAL_API_KEY")]
#[case::cohere(OcrConfigKind::Cohere, "COHERE_API_KEY")]
#[case::azure_ai(OcrConfigKind::AzureAi, "AZURE_AI_API_KEY")]
#[case::azure_cohere(OcrConfigKind::AzureCohere, "AZURE_AI_API_KEY")]
#[case::azure_document_intelligence(
    OcrConfigKind::AzureDocumentIntelligence,
    "AZURE_DOCUMENT_INTELLIGENCE_API_KEY"
)]
#[case::reducto(OcrConfigKind::ReductoV3, "REDUCTO_API_KEY")]
#[case::vertex_ai(OcrConfigKind::VertexAi, "VERTEX_AI_API_KEY")]
#[case::vertex_deepseek(OcrConfigKind::VertexDeepSeek, "VERTEX_AI_API_KEY")]
fn forward_secret_names_include_the_provider_key(#[case] kind: OcrConfigKind, #[case] name: &str) {
    assert!(kind.secret_names().contains(&name));
}

#[rstest]
#[case::mistral("mistral/ocr", Some("MISTRAL_API_KEY"))]
#[case::cohere("cohere/parse", Some("COHERE_API_KEY"))]
#[case::azure_ai("azure_ai/ocr", Some("AZURE_AI_API_KEY"))]
#[case::azure_cohere("azure_ai/cohere-parse", Some("AZURE_AI_API_KEY"))]
#[case::azure_document_intelligence(
    "azure_ai/doc-intelligence/prebuilt-layout",
    Some("AZURE_DOCUMENT_INTELLIGENCE_API_KEY")
)]
#[case::reducto_v3("reducto/parse-v3", None)]
#[case::reducto_legacy("reducto/parse-legacy", None)]
#[case::textract("aws_textract/detect-document-text", None)]
#[case::vertex_ai("vertex_ai/mistral-ocr", Some("VERTEX_AI_API_KEY"))]
#[case::vertex_deepseek("vertex_ai/deepseek-ocr", Some("VERTEX_AI_API_KEY"))]
fn forward_api_key_env_var_follows_the_selected_config(
    #[case] model: &str,
    #[case] expected: Option<&str>,
) {
    assert_eq!(
        get_api_key_env_var(model, None).expect("the provider resolves"),
        expected
    );
}

#[test]
fn forward_api_key_env_var_follows_an_explicit_provider() {
    assert_eq!(
        get_api_key_env_var("mistral-ocr-latest", Some("vertex_ai"))
            .expect("the provider resolves"),
        Some("VERTEX_AI_API_KEY")
    );
}

#[rstest]
#[case::mistral("mistral/ocr", "application", "pdf", b"%PDF-")]
#[case::azure_ai("azure_ai/ocr", "application", "pdf", b"%PDF-")]
#[case::azure_document_intelligence(
    "azure_ai/doc-intelligence/prebuilt-layout",
    "application",
    "pdf",
    b"%PDF-"
)]
#[case::reducto_v3("reducto/parse-v3", "application", "pdf", b"%PDF-")]
#[case::reducto_legacy("reducto/parse-legacy", "application", "pdf", b"%PDF-")]
#[case::vertex_ai("vertex_ai/mistral-ocr", "application", "pdf", b"%PDF-")]
#[case::vertex_deepseek("vertex_ai/deepseek-ocr", "application", "pdf", b"%PDF-")]
#[case::cohere("cohere/parse", "image", "png", b"\x89PNG\r\n\x1a\n")]
#[case::azure_cohere("azure_ai/cohere-parse", "image", "png", b"\x89PNG\r\n\x1a\n")]
#[case::textract_detect(
    "aws_textract/detect-document-text",
    "image",
    "png",
    b"\x89PNG\r\n\x1a\n"
)]
#[case::textract_analyze("aws_textract/analyze-document", "image", "png", b"\x89PNG\r\n\x1a\n")]
fn forward_health_check_documents_use_the_selected_config_media_type(
    #[case] model: &str,
    #[case] media_type: &str,
    #[case] subtype: &str,
    #[case] magic: &[u8],
) {
    let document = get_health_check_document(model, None).expect("the document resolves");
    let inline = InlineDocument::parse(document.source())
        .expect("the data URI parses")
        .expect("the document is a data URI");
    assert!(inline.0.mime_type().matches(media_type, subtype));
    assert!(
        inline
            .decode(4096)
            .expect("within the cap")
            .starts_with(magic)
    );
}

#[rstest]
#[case(OcrConfigKind::AwsTextract)]
#[case(OcrConfigKind::AwsTextractAnalyze)]
#[case(OcrConfigKind::Cohere)]
#[case(OcrConfigKind::Mistral)]
#[case(OcrConfigKind::AzureAi)]
#[case(OcrConfigKind::AzureCohere)]
#[case(OcrConfigKind::AzureDocumentIntelligence)]
#[case(OcrConfigKind::ReductoLegacy)]
#[case(OcrConfigKind::ReductoV3)]
#[case(OcrConfigKind::VertexAi)]
#[case(OcrConfigKind::VertexDeepSeek)]
fn forward_resolve_connection_params_prefers_dynamic_values(#[case] kind: OcrConfigKind) {
    let inputs = OcrCredentialInputs {
        api_key: Some(OcrSourced {
            value: SecretString::from("static-key"),
            source: OcrInputSource::Deployment,
        }),
        dynamic_api_key: Some(OcrSourced {
            value: SecretString::from("dynamic-key"),
            source: OcrInputSource::Environment,
        }),
        api_base: Some(OcrSourced {
            value: "https://static.test".to_owned(),
            source: OcrInputSource::Deployment,
        }),
        dynamic_api_base: Some(OcrSourced {
            value: "https://dynamic.test".to_owned(),
            source: OcrInputSource::Request,
        }),
    };
    let resolved = kind.resolve_connection_params(inputs);
    let api_key = resolved.api_key.expect("the dynamic key wins");
    assert_eq!(api_key.value.expose_secret(), "dynamic-key");
    assert_eq!(api_key.source, OcrInputSource::Environment);
    let api_base = resolved.api_base.expect("the dynamic base wins");
    assert_eq!(api_base.value, "https://dynamic.test");
    assert_eq!(api_base.source, OcrInputSource::Request);
}

#[rstest]
#[case::mistral("azure_ai/mistral-document-ai-2512", "providers/mistral/azure/ocr")]
#[case::mistral_with_slashes("azure_ai/mistral-document-ai-2512", "/providers/mistral/azure/ocr/")]
#[case::cohere("azure_ai/Cohere-parse-v5", "providers/cohere/v2/parse")]
#[case::document_intelligence(
    "azure_ai/doc-intelligence/prebuilt-layout",
    "documentintelligence/documentModels/prebuilt-layout:analyze"
)]
#[case::bare_mistral("mistral-document-ai-2512", "providers/mistral/azure/ocr")]
#[case::bare_cohere("Cohere-parse-v5", "providers/cohere/v2/parse")]
#[case::bare_document_intelligence(
    "doc-intelligence/prebuilt-layout",
    "documentintelligence/documentModels/prebuilt-layout:analyze"
)]
fn passthrough_recognizes_the_resolved_config_ocr_route(
    #[case] model: &str,
    #[case] endpoint: &str,
) {
    let error =
        passthrough_response(model, endpoint, b"not json").expect_err("the route is an OCR route");
    assert!(matches!(error, OcrError::ResponseField { .. }));
}

#[rstest]
#[case::other_route("azure_ai/mistral-document-ai-2512", "models/info")]
#[case::cohere_route_for_mistral("azure_ai/mistral-document-ai-2512", "providers/cohere/v2/parse")]
#[case::mistral_route_for_cohere("azure_ai/Cohere-parse-v5", "providers/mistral/azure/ocr")]
#[case::other_document_model(
    "azure_ai/doc-intelligence/prebuilt-layout",
    "documentintelligence/documentModels/prebuilt-read:analyze"
)]
#[case::empty_endpoint("azure_ai/mistral-document-ai-2512", "")]
#[case::bare_mistral_other_route("mistral-document-ai-2512", "models/info")]
fn passthrough_skips_other_routes(#[case] model: &str, #[case] endpoint: &str) {
    assert!(
        passthrough_response(model, endpoint, b"not json")
            .expect("the route resolves")
            .is_none()
    );
}

#[test]
fn passthrough_normalizes_the_mistral_body() {
    let response = passthrough_response(
        "azure_ai/mistral-document-ai-2512",
        "providers/mistral/azure/ocr",
        MISTRAL_PASSTHROUGH_BODY,
    )
    .expect("the route resolves")
    .expect("the route is an OCR route");
    assert!(response.provider_native_response.is_none());
    let json = serde_json::to_value(&response).expect("the response serializes");
    assert_eq!(json["usage_info"]["pages_processed"], 2_i32);
    assert_eq!(json["pages"][0]["markdown"], "page one");
    assert_eq!(json["model"], "mistral-document-ai-2512");
}

#[rstest]
#[case::mistral(
    "mistral-document-ai-2512",
    "providers/mistral/azure/ocr",
    MISTRAL_PASSTHROUGH_BODY,
    2_i32,
    "mistral-document-ai-2512"
)]
#[case::cohere(
    "Cohere-parse-v5",
    "providers/cohere/v2/parse",
    COHERE_PASSTHROUGH_BODY,
    3_i32,
    "Cohere-parse-v5"
)]
#[case::document_intelligence(
    "doc-intelligence/prebuilt-layout",
    "documentintelligence/documentModels/prebuilt-layout:analyze",
    DOCUMENT_INTELLIGENCE_PASSTHROUGH_BODY,
    1_i32,
    "doc-intelligence/prebuilt-layout"
)]
fn passthrough_resolves_a_bare_model_as_azure_ai(
    #[case] model: &str,
    #[case] endpoint: &str,
    #[case] body: &[u8],
    #[case] expected_pages: i32,
    #[case] expected_model: &str,
) {
    let response = passthrough_response(model, endpoint, body)
        .expect("the projector resolves a bare model")
        .expect("the route is an OCR route");
    let json = serde_json::to_value(&response).expect("the response serializes");
    assert_eq!(json["usage_info"]["pages_processed"], expected_pages);
    assert_eq!(json["model"], expected_model);
}

#[test]
fn passthrough_normalizes_the_cohere_body() {
    let response = passthrough_response(
        "azure_ai/Cohere-parse-v5",
        "providers/cohere/v2/parse",
        COHERE_PASSTHROUGH_BODY,
    )
    .expect("the route resolves")
    .expect("the route is an OCR route");
    let json = serde_json::to_value(&response).expect("the response serializes");
    assert_eq!(json["usage_info"]["pages_processed"], 3_i32);
    assert_eq!(json["model"], "Cohere-parse-v5");
}

#[test]
fn passthrough_normalizes_the_document_intelligence_body() {
    let response = passthrough_response(
        "azure_ai/doc-intelligence/prebuilt-layout",
        "documentintelligence/documentModels/prebuilt-layout:analyze",
        DOCUMENT_INTELLIGENCE_PASSTHROUGH_BODY,
    )
    .expect("the route resolves")
    .expect("the route is an OCR route");
    let json = serde_json::to_value(&response).expect("the response serializes");
    assert_eq!(json["usage_info"]["pages_processed"], 1_i32);
    assert_eq!(json["pages"][0]["index"], 0_i32);
    assert_eq!(json["pages"][0]["markdown"], "line one");
    assert_eq!(json["content"], "line one");
}

#[test]
fn passthrough_rejects_a_non_azure_kind() {
    let error = azure_ocr_route(
        OcrConfigKind::Mistral,
        "mistral-ocr-latest",
        "providers/mistral/azure/ocr",
    )
    .expect_err("a Mistral kind has no passthrough route");
    assert!(matches!(&error, OcrError::InvalidProvider(name) if name == "mistral"));
    let vertex_error = azure_ocr_route(OcrConfigKind::VertexDeepSeek, "deepseek-ocr", "")
        .expect_err("a Vertex kind has no passthrough route");
    assert!(matches!(&vertex_error, OcrError::InvalidProvider(name) if name == "vertex_ai"));
}

#[tokio::test]
async fn perform_rewrites_the_wire_request_and_sees_the_response_body_once() {
    let server = MockServer::start(vec![reply("200 OK", &[], MISTRAL_WIRE_BODY)]).await;
    let client = test_client();
    let interceptors = RecordingInterceptors::default();
    let response = perform_ocr_request(
        &client,
        OcrConfigKind::Mistral,
        mistral_request(&server.base),
        &interceptors,
    )
    .await
    .expect("the call succeeds");
    assert_eq!(response.pages.len(), 1);
    assert_eq!(response.pages[0].markdown, "hello");
    assert_eq!(
        interceptors.response_bodies(),
        vec![MISTRAL_WIRE_BODY.to_vec()]
    );
    let requests = server.requests().await;
    let sent = requests.first().expect("one request is sent");
    assert!(sent.contains("\"extract_header\":true"), "{sent}");
    assert!(sent.to_ascii_lowercase().contains("x-hook: seen"), "{sent}");
}

#[tokio::test]
async fn perform_carries_the_call_context_to_the_interceptor() {
    let server = MockServer::start(vec![reply("200 OK", &[], MISTRAL_WIRE_BODY)]).await;
    let client = test_client();
    let interceptors = RecordingInterceptors::default();
    let mut request = mistral_request(&server.base);
    request.optional_params = context_params();
    perform_ocr_request(&client, OcrConfigKind::Mistral, request, &interceptors)
        .await
        .expect("the call succeeds");
    let contexts = interceptors.call_contexts();
    let context = contexts.first().expect("the interceptor sees the context");
    assert_eq!(context.model, "mistral-ocr-latest");
    assert_eq!(context.provider, "mistral");
    assert_eq!(
        context.secret_fields,
        SECRET_OPTION_NAMES.map(str::to_owned).to_vec()
    );
    assert_eq!(context.api_key.as_deref(), Some("sk-test"));
    assert_eq!(
        context.optional_params.get("pages"),
        Some(&Json::List(vec![
            Json::Number(serde_json::Number::from(0_i64)),
            Json::Number(serde_json::Number::from(2_i64))
        ]))
    );
}

#[tokio::test]
async fn perform_resolves_secrets_from_the_client_source() {
    let server = MockServer::start(vec![reply("200 OK", &[], MISTRAL_WIRE_BODY)]).await;
    let mut client = test_client();
    client.secrets = Arc::new(FixedSecrets(BTreeMap::from([(
        "MISTRAL_API_KEY".to_owned(),
        SecretString::from("sk-from-source"),
    )])));
    let interceptors = RecordingInterceptors::default();
    perform_ocr_request(
        &client,
        OcrConfigKind::Mistral,
        prepared_request(server.base.clone(), Object::new()),
        &interceptors,
    )
    .await
    .expect("the call succeeds");
    assert_eq!(interceptors.response_bodies().len(), 1);
    let requests = server.requests().await;
    let sent = requests.first().expect("one request is sent");
    assert!(
        sent.to_ascii_lowercase()
            .contains("authorization: bearer sk-from-source"),
        "{sent}"
    );
}

#[tokio::test]
async fn perform_maps_a_secret_lookup_failure_to_secret() {
    let server = MockServer::start(Vec::new()).await;
    let mut client = test_client();
    client.secrets = Arc::new(FailingSecrets);
    let error = perform_ocr_request(
        &client,
        OcrConfigKind::Mistral,
        mistral_request(&server.base),
        &(),
    )
    .await
    .expect_err("a failed secret lookup fails the call");
    assert!(
        matches!(&error, OcrError::Secret(source) if source.to_string().contains("unreachable"))
    );
    assert!(server.requests().await.is_empty());
}

#[tokio::test]
async fn perform_rejects_an_invalid_request_format_before_the_call() {
    let server = MockServer::start(Vec::new()).await;
    let mut client = test_client();
    client.secrets = Arc::new(FailingSecrets);
    let mut request = mistral_request(&server.base);
    request.optional_params = object(&[("req_format", Json::Text("bogus".to_owned()))]);
    let error = perform_ocr_request(&client, OcrConfigKind::Mistral, request, &())
        .await
        .expect_err("the invalid format fails the call");
    assert!(matches!(error, OcrError::RequestFormat));
    assert!(server.requests().await.is_empty());
}

#[tokio::test]
async fn perform_dispatches_cohere_models_to_the_cohere_config() {
    let body = br#"{"pages": [{"index": 0, "markdown": {"content": "hello"}}], "meta": {"billed_units": {"pages": 1}}}"#;
    let server = MockServer::start(vec![reply("200 OK", &[], body)]).await;
    let client = test_client();
    let mut request = prepared_request(server.base.clone(), Object::new());
    request.model = "parse".to_owned();
    request.document = OcrDocument::ImageUrl {
        image_url: COHERE_HEALTH_CHECK_DATA_URI.to_owned(),
        extra: BTreeMap::new(),
    };
    request.connection.api_key = Some(SecretString::from("sk-test"));
    let response = perform_ocr_request(&client, OcrConfigKind::Cohere, request, &())
        .await
        .expect("the Cohere call succeeds");
    assert_eq!(response.pages[0].markdown, "hello");
    let requests = server.requests().await;
    let sent = requests.first().expect("one request is sent");
    assert!(sent.starts_with("POST /v2/parse HTTP/1.1"), "{sent}");
    assert!(sent.contains("\"model\":\"parse\""), "{sent}");
    assert!(sent.contains("\"output_format\":\"markdown\""), "{sent}");
}
