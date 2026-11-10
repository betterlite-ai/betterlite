#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use rstest::rstest;
use secrecy::SecretString;
use tokio::time::timeout;

use crate::json::{Json, Object};
use crate::ocr_auth::{VertexAccessToken, VertexTokenSource};
use crate::ocr_config::connection::{
    OcrConnection, OcrInputSource, OcrSourced, OcrTransportConfig, ResolvedOcrCredentials,
};
use crate::ocr_config::{
    OCR_RESPONSE_MAX_BYTES, OcrProviderConfig as _, OcrTransform, PreparedOcrRequest, Secrets,
};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::{OcrAuthError, OcrError};
use crate::ocr_http::tests::support::{MockServer, ocr_client, reply};
use crate::ocr_http::{CallHooks, OcrClient, OcrWireRequest, ocr};
use crate::ocr_settings::OcrSettings;
use crate::request_format::RequestFormat;
use crate::response::OCRResponse;

use super::url::chat_completions_url;
use super::{DeepSeekOcrRequest, VertexAIDeepSeekOCRConfig};

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

struct TokenSource {
    token: &'static str,
    project: Option<&'static str>,
    calls: Arc<AtomicUsize>,
}

impl TokenSource {
    fn fake(
        token: &'static str,
        project: Option<&'static str>,
    ) -> (Arc<dyn VertexTokenSource>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let source: Arc<dyn VertexTokenSource> = Arc::new(Self {
            token,
            project,
            calls: Arc::clone(&calls),
        });
        (source, calls)
    }
}

#[async_trait]
impl VertexTokenSource for TokenSource {
    async fn token(&self, _request: &PreparedOcrRequest) -> Result<VertexAccessToken, OcrError> {
        _ = self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(VertexAccessToken {
            token: SecretString::from(self.token),
            project: self.project.map(str::to_owned),
        })
    }
}

fn client(source: &Arc<dyn VertexTokenSource>) -> OcrClient {
    let mut client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    client.vertex = Arc::clone(source);
    client
}

fn settings(project: Option<&str>, location: Option<&str>) -> OcrSettings {
    OcrSettings {
        request_timeout: Duration::from_secs(30),
        max_download_bytes: 1024,
        poll_timeout: Duration::from_secs(5),
        doc_intelligence_api_version: "2024-11-30".to_owned(),
        doc_intelligence_dpi: 96,
        vertex_project: project.map(str::to_owned),
        vertex_location: location.map(str::to_owned),
        enable_azure_ad_token_refresh: false,
    }
}

fn connection(api_base: (&str, OcrInputSource), settings: OcrSettings) -> OcrConnection {
    OcrConnection::new(
        ResolvedOcrCredentials {
            api_key: None,
            api_base: Some(OcrSourced {
                value: api_base.0.to_owned(),
                source: api_base.1,
            }),
        },
        OcrTransportConfig {
            extra_headers: Vec::new(),
            extra_headers_source: OcrInputSource::Deployment,
            timeout: None,
            max_response_bytes: OCR_RESPONSE_MAX_BYTES,
        },
        settings,
        Secrets::new(),
    )
}

fn prepared(connection: OcrConnection, optional_params: Object) -> PreparedOcrRequest {
    PreparedOcrRequest {
        model: "deepseek-ocr-maas".to_owned(),
        document: OcrDocument::DocumentUrl {
            document_url: "data:application/pdf;base64,YWJj".to_owned(),
            extra: BTreeMap::new(),
        },
        connection,
        caller_document: false,
        optional_params,
        input_sources: BTreeMap::new(),
        azure_ad_token_provider: None,
    }
}

fn params(text: &str) -> Object {
    serde_json::from_str(text).expect("the test options are an object")
}

fn json_value(text: &str) -> Json {
    serde_json::from_str(text).expect("the test value is JSON")
}

fn document(source: &str) -> OcrDocument {
    OcrDocument::DocumentUrl {
        document_url: source.to_owned(),
        extra: BTreeMap::new(),
    }
}

fn transform(model: &str, optional_params: &Object) -> DeepSeekOcrRequest {
    VertexAIDeepSeekOCRConfig
        .transform_ocr_request(
            document("gs://bucket/a.png"),
            OcrTransform {
                model,
                optional_params,
                headers: &[],
            },
        )
        .expect("the request transforms")
}

fn transformed(optional_params: &Object) -> Object {
    let request = transform("deepseek-ocr-maas", optional_params);
    let Json::Object(body) =
        json_value(&serde_json::to_string(&request).expect("the request is JSON"))
    else {
        panic!("the request is an object");
    };
    body
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
    serde_json::from_str(body).expect("the body is JSON")
}

fn decode(payload: &str) -> Result<OCRResponse, OcrError> {
    VertexAIDeepSeekOCRConfig.transform_ocr_response(
        "requested-model",
        payload.as_bytes(),
        RequestFormat::Litellm,
    )
}

#[rstest]
#[case::default_base(
    None,
    "proj-1",
    "europe-west4",
    "https://aiplatform.googleapis.com/v1/projects/proj-1/locations/europe-west4/endpoints/openapi/chat/completions"
)]
#[case::deployment_base(
    Some("https://proxy.example"),
    "p",
    "us-central1",
    "https://proxy.example/v1/projects/p/locations/us-central1/endpoints/openapi/chat/completions"
)]
#[case::base_with_the_first_segment(
    Some("https://proxy.example/v1"),
    "p",
    "us-central1",
    "https://proxy.example/v1/projects/p/locations/us-central1/endpoints/openapi/chat/completions"
)]
#[case::base_query_is_kept(
    Some("https://proxy.example?tenant=a"),
    "p",
    "us-central1",
    "https://proxy.example/v1/projects/p/locations/us-central1/endpoints/openapi/chat/completions?tenant=a"
)]
#[case::base_is_trimmed(
    Some("  https://proxy.example  "),
    "p",
    "us-central1",
    "https://proxy.example/v1/projects/p/locations/us-central1/endpoints/openapi/chat/completions"
)]
#[case::blank_base_uses_the_default(
    Some("   "),
    "p",
    "us-central1",
    "https://aiplatform.googleapis.com/v1/projects/p/locations/us-central1/endpoints/openapi/chat/completions"
)]
fn builds_the_vertex_chat_completions_url(
    #[case] api_base: Option<&str>,
    #[case] project: &str,
    #[case] location: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        chat_completions_url(api_base, project, location).expect("the URL builds"),
        expected
    );
}

#[rstest]
#[case::not_a_url("not a url")]
#[case::cannot_be_a_base("mailto:ops@example.com")]
fn an_api_base_that_does_not_parse_is_the_api_base_request_field(#[case] api_base: &str) {
    let error = chat_completions_url(Some(api_base), "proj", "us-central1")
        .expect_err("the base does not parse");
    assert!(
        matches!(&error, OcrError::RequestField { path } if path == "api_base"),
        "{error:?}"
    );
}

#[test]
fn sends_the_document_as_an_image_url_content_part() {
    let body = transformed(&Object::new());
    let Some(Json::List(messages)) = body.get("messages") else {
        panic!("the request carries messages");
    };
    assert_eq!(messages.len(), 1);
    let Some(Json::Object(message)) = messages.first() else {
        panic!("the message is an object");
    };
    assert_eq!(message.get("role"), Some(&Json::Text("user".to_owned())));
    let Some(Json::List(content)) = message.get("content") else {
        panic!("the message carries content parts");
    };
    assert_eq!(content.len(), 1);
    assert_eq!(
        content.first(),
        Some(&json_value(
            r#"{"type":"image_url","image_url":"gs://bucket/a.png"}"#
        ))
    );
}

#[rstest]
#[case::plain("deepseek-ocr-maas", "deepseek-ai/deepseek-ocr-maas")]
#[case::prefixed("deepseek-ai/deepseek-ocr-maas", "deepseek-ai/deepseek-ocr-maas")]
#[case::twice_prefixed("deepseek-ai/deepseek-ai/deepseek-ocr", "deepseek-ai/deepseek-ocr")]
fn prefixes_the_model_with_the_deepseek_namespace(#[case] model: &str, #[case] expected: &str) {
    assert_eq!(transform(model, &Object::new()).model, expected);
}

#[rstest]
#[case::only_the_prefix("deepseek-ai/")]
#[case::empty("")]
fn a_model_without_a_name_is_the_model_request_field(#[case] model: &str) {
    let error = VertexAIDeepSeekOCRConfig
        .transform_ocr_request(
            document("gs://bucket/a.png"),
            OcrTransform {
                model,
                optional_params: &Object::new(),
                headers: &[],
            },
        )
        .expect_err("the model has no name");
    assert!(
        matches!(&error, OcrError::RequestField { path } if path == "model"),
        "{error:?}"
    );
}

#[test]
fn a_missing_document_url_is_missing_document_url() {
    let error = VertexAIDeepSeekOCRConfig
        .transform_ocr_request(
            document(""),
            OcrTransform {
                model: "deepseek-ocr-maas",
                optional_params: &Object::new(),
                headers: &[],
            },
        )
        .expect_err("the document has no URL");
    assert!(matches!(&error, OcrError::MissingDocumentUrl), "{error:?}");
}

#[rstest]
#[case::stream("stream", "true")]
#[case::temperature("temperature", "0.1")]
#[case::max_tokens("max_tokens", "1024")]
#[case::top_p("top_p", "0.9")]
#[case::n("n", "2")]
#[case::stop_text("stop", r#""done""#)]
#[case::stop_list("stop", r#"["done","stop"]"#)]
#[case::explicit_null("temperature", "null")]
fn copies_a_supported_option_into_the_request(#[case] name: &str, #[case] value: &str) {
    let body = transformed(&params(&format!(r#"{{"{name}":{value}}}"#)));
    assert_eq!(body.get(name), Some(&json_value(value)));
}

#[test]
fn an_option_the_endpoint_does_not_take_stays_out_of_the_request() {
    let body = transformed(&params(r#"{"ignored":true}"#));
    assert!(body.get("ignored").is_none());
}

#[test]
fn sends_greedy_sampling_defaults_unless_the_caller_overrides_them() {
    let defaults = transformed(&Object::new());
    assert_eq!(defaults.get("temperature"), Some(&json_value("0.0")));
    assert_eq!(
        defaults.get("repetition_penalty"),
        Some(&json_value("1.05"))
    );
    let overridden = transformed(&params(r#"{"temperature":0.7}"#));
    assert_eq!(overridden.get("temperature"), Some(&json_value("0.7")));
    assert_eq!(
        overridden.get("repetition_penalty"),
        Some(&json_value("1.05"))
    );
}

const STRUCTURED_CONTENT: &str = r#"{"choices":[{"message":{"content":{"pages":[{"index":2,"markdown":"page","images":[{"id":"one"}],"dimensions":{"width":10},"page_extra":"kept"}],"model":"provider-model","usage_info":{"pages_processed":1},"document_annotation":{"language":"en"},"tables":[{"rows":2}],"keyValuePairs":[{"k":"v"}],"future":"kept"}}}],"usage":{"prompt_tokens":0}}"#;

#[test]
fn decodes_the_pages_from_the_chat_content() {
    let response = decode(STRUCTURED_CONTENT).expect("the response decodes");
    assert_eq!(response.object, "ocr");
    assert_eq!(response.pages.len(), 1);
    assert_eq!(response.pages[0].index, 2);
    assert_eq!(response.pages[0].markdown, "page");
    assert_eq!(
        response.pages[0]
            .images
            .as_ref()
            .and_then(|images| images.first())
            .and_then(|image| image.extra.get("id")),
        Some(&Json::Text("one".to_owned()))
    );
    assert_eq!(
        response.pages[0]
            .dimensions
            .as_ref()
            .and_then(|dimensions| dimensions.width),
        Some(10)
    );
    assert_eq!(
        response.pages[0].extra.get("page_extra"),
        Some(&Json::Text("kept".to_owned()))
    );
}

#[test]
fn decodes_the_metadata_from_the_chat_content() {
    let response = decode(STRUCTURED_CONTENT).expect("the response decodes");
    assert_eq!(response.model, "provider-model");
    assert_eq!(
        response
            .usage_info
            .as_ref()
            .and_then(|usage| usage.pages_processed),
        Some(1)
    );
    assert_eq!(
        response.document_annotation,
        Some(json_value(r#"{"language":"en"}"#))
    );
    assert_eq!(response.tables, Some(vec![params(r#"{"rows":2}"#)]));
    assert_eq!(response.key_value_pairs, Some(vec![params(r#"{"k":"v"}"#)]));
    assert_eq!(
        response.extra.get("future"),
        Some(&Json::Text("kept".to_owned()))
    );
}

#[rstest]
#[case::plain_text(r##"{"choices":[{"message":{"content":"# hello"}}]}"##, "# hello")]
#[case::broken_json(r#"{"choices":[{"message":{"content":"{broken"}}]}"#, "{broken")]
#[case::json_list_text(r#"{"choices":[{"message":{"content":"[]"}}]}"#, "[]")]
#[case::json_text(
    r#"{"choices":[{"message":{"content":"{\"pages\":[{\"markdown\":\"json text\"}]}"}}]}"#,
    "json text"
)]
fn a_text_content_becomes_a_page(#[case] payload: &str, #[case] markdown: &str) {
    let response = decode(payload).expect("the response decodes");
    assert_eq!(response.pages.len(), 1);
    assert_eq!(response.pages[0].index, 0);
    assert_eq!(response.pages[0].markdown, markdown);
    assert_eq!(response.model, "requested-model");
}

#[test]
fn an_empty_pages_array_keeps_the_raw_text_as_the_page() {
    let response = decode(r#"{"choices":[{"message":{"content":" {\"pages\":[]} "}}]}"#)
        .expect("the response decodes");
    assert_eq!(response.pages.len(), 1);
    assert_eq!(response.pages[0].markdown, " {\"pages\":[]} ");
    assert!(response.usage_info.is_none());
}

#[test]
fn object_content_without_pages_becomes_one_page_of_standard_json() {
    let payload = r#"{"choices":[{"message":{"content":{"text":"hello","n":1}}}],"usage":{"prompt_tokens":1}}"#;
    let response = decode(payload).expect("the response decodes");
    assert_eq!(response.pages.len(), 1);
    assert_eq!(response.pages[0].markdown, r#"{"text":"hello","n":1}"#);
    assert_eq!(
        response
            .usage_info
            .as_ref()
            .and_then(|usage| usage.extra.get("prompt_tokens")),
        Some(&json_value("1"))
    );
}

#[rstest]
#[case::no_choices(r#"{"choices":[]}"#)]
#[case::missing_content(r#"{"choices":[{"message":{}}]}"#)]
#[case::null_content(r#"{"choices":[{"message":{"content":null}}]}"#)]
#[case::empty_text(r#"{"choices":[{"message":{"content":""}}]}"#)]
#[case::empty_object(r#"{"choices":[{"message":{"content":{}}}]}"#)]
fn empty_content_is_empty_content(#[case] payload: &str) {
    let error = decode(payload).expect_err("there is no content");
    assert!(matches!(&error, OcrError::EmptyContent), "{error:?}");
}

#[rstest]
#[case::text_pages(r#"{"choices":[{"message":{"content":{"pages":"x"}}}]}"#)]
#[case::null_pages(r#"{"choices":[{"message":{"content":{"pages":null}}}]}"#)]
#[case::number_pages(r#"{"choices":[{"message":{"content":{"pages":42}}}]}"#)]
fn a_bad_pages_value_is_the_pages_response_field(#[case] payload: &str) {
    let error = decode(payload).expect_err("pages is not an array");
    assert!(
        matches!(&error, OcrError::ResponseField { path } if path == "choices[0].message.content.pages"),
        "{error:?}"
    );
}

#[test]
fn a_bad_model_value_is_the_model_response_field() {
    let error = decode(r#"{"choices":[{"message":{"content":{"pages":[],"model":42}}}]}"#)
        .expect_err("model is not a string");
    assert!(
        matches!(&error, OcrError::ResponseField { path } if path == "choices[0].message.content.model"),
        "{error:?}"
    );
}

#[rstest]
#[case::markdown(r#"{"pages":[{"markdown":42}]}"#)]
#[case::index(r#"{"pages":[{"index":"abc"}]}"#)]
#[case::index_null(r#"{"pages":[{"index":null}]}"#)]
#[case::index_fraction(r#"{"pages":[{"index":2.5}]}"#)]
#[case::index_fraction_string(r#"{"pages":[{"index":"2.5"}]}"#)]
#[case::index_trailing_separator(r#"{"pages":[{"index":"2_"}]}"#)]
#[case::index_signed_separator(r#"{"pages":[{"index":"+_2"}]}"#)]
fn a_bad_page_field_names_its_page_path(#[case] content: &str) {
    let payload = r#"{"choices":[{"message":{"content":CONTENT}}]}"#.replace("CONTENT", content);
    let error = decode(&payload).expect_err("the page does not decode");
    assert!(
        matches!(
            &error,
            OcrError::ResponseField { path }
                if path.starts_with("choices[0].message.content.pages[0]")
        ),
        "{error:?}"
    );
}

#[test]
fn a_page_index_accepts_the_lax_integer_spellings() {
    let payload = r#"{"choices":[{"message":{"content":{"pages":[{"index":"2","markdown":"a"},{"index":3.0,"markdown":"b"},{"index":"4.0","markdown":"c"},{"index":"1_0","markdown":"d"},{"markdown":"e"}],"usage_info":null}}}]}"#;
    let response = decode(payload).expect("the response decodes");
    assert_eq!(response.pages.len(), 5);
    assert_eq!(response.pages[0].index, 2);
    assert_eq!(response.pages[1].index, 3);
    assert_eq!(response.pages[2].index, 4);
    assert_eq!(response.pages[3].index, 10);
    assert_eq!(response.pages[4].index, 0);
    assert!(response.usage_info.is_none());
}

#[test]
fn a_page_that_is_not_an_object_is_skipped() {
    let payload =
        r#"{"choices":[{"message":{"content":{"pages":[42,{"index":1,"markdown":"kept"}]}}}]}"#;
    let response = decode(payload).expect("the response decodes");
    assert_eq!(response.pages.len(), 1);
    assert_eq!(response.pages[0].index, 1);
}

#[test]
fn the_chat_usage_fills_usage_info_when_the_content_has_no_pages() {
    let payload = r#"{"choices":[{"message":{"content":"plain"}}],"usage":{"prompt_tokens":3}}"#;
    let response = decode(payload).expect("the response decodes");
    assert_eq!(
        response
            .usage_info
            .as_ref()
            .and_then(|usage| usage.extra.get("prompt_tokens")),
        Some(&json_value("3"))
    );
}

#[test]
fn the_chat_usage_is_ignored_when_the_content_has_pages() {
    let payload =
        r#"{"choices":[{"message":{"content":{"pages":[]}}}],"usage":{"prompt_tokens":3}}"#;
    let response = decode(payload).expect("the response decodes");
    assert!(response.usage_info.is_none());
}

fn assert_chat_request(sent: &str) {
    assert!(
        sent.starts_with(
            "POST /v1/projects/proj-9/locations/europe-west4/endpoints/openapi/chat/completions HTTP/1.1"
        ),
        "{sent}"
    );
    assert!(
        sent.to_ascii_lowercase()
            .contains("authorization: bearer ya29.token"),
        "{sent}"
    );
    let Json::Object(body) = request_body(sent) else {
        panic!("the body is an object");
    };
    assert_eq!(
        body.get("model"),
        Some(&Json::Text("deepseek-ai/deepseek-ocr-maas".to_owned()))
    );
    assert_eq!(body.get("temperature"), Some(&json_value("0.0")));
    assert_eq!(body.get("repetition_penalty"), Some(&json_value("1.05")));
}

#[tokio::test]
async fn sends_the_chat_completions_request() {
    let answer = br#"{"choices":[{"message":{"content":{"pages":[{"index":0,"markdown":"hello"}]}}}],"usage":{}}"#;
    let server = MockServer::start(vec![reply("200 OK", &[], answer)]).await;
    let (source, _calls) = TokenSource::fake("ya29.token", None);
    let response = ocr(
        &VertexAIDeepSeekOCRConfig,
        &client(&source),
        &prepared(
            connection(
                (&server.base, OcrInputSource::Deployment),
                settings(Some("proj-9"), Some("europe-west4")),
            ),
            Object::new(),
        ),
        &NoHooks,
    )
    .await
    .expect("the call succeeds");
    assert_eq!(response.pages[0].markdown, "hello");
    let requests = recorded(server).await;
    assert_eq!(requests.len(), 1);
    assert_chat_request(&requests[0]);
}

#[tokio::test]
async fn a_request_supplied_api_base_is_refused_before_a_request() {
    let server = MockServer::start(Vec::new()).await;
    let (source, calls) = TokenSource::fake("ya29.token", Some("proj-1"));
    let error = ocr(
        &VertexAIDeepSeekOCRConfig,
        &client(&source),
        &prepared(
            connection(
                (&server.base, OcrInputSource::Request),
                settings(Some("proj-1"), None),
            ),
            Object::new(),
        ),
        &NoHooks,
    )
    .await
    .expect_err("a request api_base is refused");
    assert!(
        matches!(
            error,
            OcrError::Auth(OcrAuthError::InvalidConfiguration { .. })
        ),
        "{error:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(recorded(server).await.is_empty());
}

#[test]
fn exposes_the_vertex_key_environment_and_secret_names() {
    assert_eq!(
        VertexAIDeepSeekOCRConfig.get_api_key_env_var(),
        Some("VERTEX_AI_API_KEY")
    );
    assert_eq!(
        VertexAIDeepSeekOCRConfig.secret_names(),
        [
            "VERTEX_AI_API_KEY",
            "VERTEXAI_API_KEY",
            "VERTEXAI_CREDENTIALS",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "VERTEXAI_PROJECT",
            "VERTEXAI_LOCATION",
            "VERTEX_LOCATION",
        ]
    );
    assert!(
        VertexAIDeepSeekOCRConfig
            .get_supported_ocr_params("deepseek-ocr-maas")
            .is_empty()
    );
    assert_eq!(
        VertexAIDeepSeekOCRConfig
            .map_ocr_params(&params(r#"{"temperature":0.5}"#), "deepseek-ocr-maas")
            .expect("the options map"),
        Object::new()
    );
}
