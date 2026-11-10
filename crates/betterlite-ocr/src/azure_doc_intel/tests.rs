#![cfg(test)]

mod poll;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use rstest::rstest;
use secrecy::SecretString;

use super::options::map_params;
use super::{
    AZURE_DI_API_KEY_ENV, AZURE_DI_ENDPOINT_ENV, AZURE_DI_SUBSCRIPTION_HEADER,
    AzureDocIntelligenceOcrConfig,
};
use crate::json::{Json, Object};
use crate::ocr_auth::AzureTokenSource;
use crate::ocr_config::connection::{
    OcrConnection, OcrInputSource, OcrSourced, OcrTransportConfig, ResolvedOcrCredentials,
};
use crate::ocr_config::decode::to_json;
use crate::ocr_config::{
    OCR_RESPONSE_MAX_BYTES, OcrProviderConfig as _, OcrTransform, PreparedOcrRequest, Secrets,
};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::{OcrAuthError, OcrError};
use crate::ocr_http::tests::support::ocr_client;
use crate::ocr_http::{CallHooks, OcrClient, OcrWireRequest};
use crate::ocr_settings::OcrSettings;
use crate::request_format::RequestFormat;
use crate::response::{OCRPage, OCRResponse};

const ENDPOINT: &str = "https://doc-intel.example";

pub(super) fn document(source: &str) -> OcrDocument {
    OcrDocument::DocumentUrl {
        document_url: source.to_owned(),
        extra: BTreeMap::new(),
    }
}

pub(super) fn params(raw: &str) -> Object {
    serde_json::from_str(raw).expect("the test options are an object")
}

fn secrets_with(name: &str, value: &str) -> Secrets {
    Secrets::from([(name.to_owned(), SecretString::from(value))])
}

fn connection(
    key: Option<(&str, OcrInputSource)>,
    base: Option<(&str, OcrInputSource)>,
    extra: Vec<(&str, &str)>,
    extra_source: OcrInputSource,
    secrets: Secrets,
) -> OcrConnection {
    OcrConnection::new(
        ResolvedOcrCredentials {
            api_key: key.map(|(value, source)| OcrSourced {
                value: SecretString::from(value),
                source,
            }),
            api_base: base.map(|(value, source)| OcrSourced {
                value: value.to_owned(),
                source,
            }),
        },
        OcrTransportConfig {
            extra_headers: extra
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
            extra_headers_source: extra_source,
            timeout: None,
            max_response_bytes: OCR_RESPONSE_MAX_BYTES,
        },
        OcrSettings::default(),
        secrets,
    )
}

pub(super) fn prepared_request(connection: OcrConnection, options: Object) -> PreparedOcrRequest {
    PreparedOcrRequest {
        model: "azure_ai/doc-intelligence/prebuilt-layout".to_owned(),
        document: document("https://example.test/a.pdf"),
        connection,
        caller_document: false,
        optional_params: options,
        input_sources: BTreeMap::new(),
        azure_ad_token_provider: None,
    }
}

fn request_connection(api_base: &str, extra: Vec<(&str, &str)>) -> OcrConnection {
    connection(
        Some(("request-key", OcrInputSource::Request)),
        Some((api_base, OcrInputSource::Request)),
        extra,
        OcrInputSource::Deployment,
        Secrets::new(),
    )
}

pub(super) fn keyed_request(api_base: &str, options: Object) -> PreparedOcrRequest {
    prepared_request(request_connection(api_base, Vec::new()), options)
}

pub(super) fn plain_client() -> OcrClient {
    ocr_client(OcrSettings::default(), Duration::from_secs(5))
}

pub(super) struct NoHooks;

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

fn analyzed_url(request: &PreparedOcrRequest) -> String {
    let params = map_params(&request.optional_params).expect("the options normalize");
    AzureDocIntelligenceOcrConfig
        .get_complete_url(request, &params, &Vec::new())
        .expect("the URL completes")
}

#[rstest]
#[case::sorted(r#"{"pages":[0, 1, 2]}"#, Some("1,2,3"))]
#[case::sorted_and_deduplicated(r#"{"pages":[2, 0, 0, 1]}"#, Some("1,2,3"))]
#[case::largest_page(r#"{"pages":[9223372036854775806]}"#, Some("9223372036854775807"))]
#[case::native_range(r#"{"pages":"3-9"}"#, Some("3-9"))]
#[case::trimmed_range(r#"{"pages":"1-3, 5"}"#, Some("1-3,5"))]
#[case::string_tokens(r#"{"pages":["1", " 3-5 "]}"#, Some("1,3-5"))]
fn pages_normalize_to_query_text(#[case] options: &str, #[case] expected: Option<&str>) {
    assert_eq!(
        map_params(&params(options)).unwrap().pages.as_deref(),
        expected
    );
}

#[rstest]
#[case::empty_array(r#"{"pages":[]}"#)]
#[case::null(r#"{"pages":null}"#)]
#[case::absent("{}")]
fn pages_that_name_nothing_leave_the_query(#[case] options: &str) {
    assert_eq!(map_params(&params(options)).unwrap().pages, None);
}

#[rstest]
#[case::negative(r#"{"pages":[-1]}"#, "negative page index")]
#[case::overflow(r#"{"pages":[9223372036854775807]}"#, "page index is out of range")]
#[case::fractional(r#"{"pages":[1.0]}"#, "page index is out of range")]
#[case::unsigned_overflow(r#"{"pages":[18446744073709551615]}"#, "page index is out of range")]
#[case::mixed_number_first(r#"{"pages":[1, "2"]}"#, "expected only integers or only strings")]
#[case::mixed_string_first(r#"{"pages":["1", 2]}"#, "expected only integers or only strings")]
#[case::boolean(r#"{"pages":[true, false]}"#, "expected only integers or only strings")]
#[case::nested(r#"{"pages":[[1]]}"#, "expected only integers or only strings")]
#[case::letters(r#"{"pages":"a,b"}"#, "invalid native page range")]
#[case::triple_range(r#"{"pages":"1-3-5"}"#, "invalid native page range")]
#[case::bare_number(
    r#"{"pages":5}"#,
    "expected an array of integers or strings, or a native page range"
)]
fn invalid_pages_name_the_reason(#[case] options: &str, #[case] reason: &str) {
    let error = map_params(&params(options)).expect_err("the pages are invalid");
    assert!(matches!(error, OcrError::Pages(message) if message == reason));
}

#[rstest]
#[case::array(r#"{"features":["keyValuePairs"]}"#, "keyValuePairs")]
#[case::several(
    r#"{"features":["keyValuePairs", " languages "]}"#,
    "keyValuePairs,languages"
)]
#[case::comma_string(r#"{"features":"keyValuePairs,languages"}"#, "keyValuePairs,languages")]
#[case::trimmed_string(
    r#"{"features":"keyValuePairs, languages"}"#,
    "keyValuePairs,languages"
)]
fn features_normalize_to_query_text(#[case] options: &str, #[case] expected: &str) {
    assert_eq!(
        map_params(&params(options)).unwrap().features.as_deref(),
        Some(expected)
    );
}

#[rstest]
#[case::empty_array(r#"{"features":[]}"#)]
#[case::null(r#"{"features":null}"#)]
#[case::absent("{}")]
fn features_that_name_nothing_leave_the_query(#[case] options: &str) {
    assert_eq!(map_params(&params(options)).unwrap().features, None);
}

#[rstest]
#[case::query_injection(r#"{"features":"keyValuePairs&pages=9"}"#)]
#[case::spaces(r#"{"features":"key value pairs"}"#)]
#[case::empty_string(r#"{"features":""}"#)]
#[case::leading_digit(r#"{"features":"9x"}"#)]
#[case::numbers(r#"{"features":[1, 2]}"#)]
#[case::nested(r#"{"features":[["keyValuePairs"]]}"#)]
#[case::object(r#"{"features":{"feature":"keyValuePairs"}}"#)]
#[case::boolean(r#"{"features":true}"#)]
fn invalid_features_are_the_features_error(#[case] options: &str) {
    let error = map_params(&params(options)).expect_err("the features are invalid");
    assert!(matches!(error, OcrError::Features));
}

#[test]
fn the_analyze_path_keeps_the_last_model_segment() {
    assert_eq!(
        AzureDocIntelligenceOcrConfig::analyze_path("azure_ai/doc-intelligence/prebuilt-layout")
            .unwrap(),
        [
            "documentintelligence".to_owned(),
            "documentModels".to_owned(),
            "prebuilt-layout:analyze".to_owned(),
        ]
    );
}

#[rstest]
#[case::dot("..")]
#[case::dot_after_prefix("azure_ai/..")]
#[case::single_dot(".")]
fn a_dot_model_has_no_analyze_path(#[case] model: &str) {
    assert!(matches!(
        AzureDocIntelligenceOcrConfig::analyze_path(model),
        Err(OcrError::DotModel)
    ));
}

#[rstest]
#[case::bare_endpoint(
    "https://doc-intel.example",
    "https://doc-intel.example/documentintelligence/documentModels/prebuilt-layout:analyze?api-version=2024-11-30"
)]
#[case::trailing_slash(
    "https://doc-intel.example/",
    "https://doc-intel.example/documentintelligence/documentModels/prebuilt-layout:analyze?api-version=2024-11-30"
)]
#[case::partial_path(
    "https://doc-intel.example/documentintelligence",
    "https://doc-intel.example/documentintelligence/documentModels/prebuilt-layout:analyze?api-version=2024-11-30"
)]
#[case::complete_path(
    "https://doc-intel.example/documentintelligence/documentModels",
    "https://doc-intel.example/documentintelligence/documentModels/prebuilt-layout:analyze?api-version=2024-11-30"
)]
#[case::query_is_kept(
    "https://doc-intel.example?tenant=a",
    "https://doc-intel.example/documentintelligence/documentModels/prebuilt-layout:analyze?tenant=a&api-version=2024-11-30"
)]
fn the_analyze_url_completes_the_endpoint(#[case] base: &str, #[case] expected: &str) {
    assert_eq!(analyzed_url(&keyed_request(base, Object::new())), expected);
}

#[test]
fn the_analyze_url_carries_pages_then_features() {
    let request = keyed_request(
        ENDPOINT,
        params(r#"{"pages":[0,2],"features":["keyValuePairs","languages"],"req_format":"native"}"#),
    );
    assert_eq!(
        analyzed_url(&request),
        "https://doc-intel.example/documentintelligence/documentModels/prebuilt-layout:analyze?api-version=2024-11-30&pages=1%2C3&features=keyValuePairs%2Clanguages"
    );
}

#[test]
fn a_blank_base_falls_back_to_the_endpoint_secret() {
    let mut request = prepared_request(
        connection(
            None,
            None,
            Vec::new(),
            OcrInputSource::Deployment,
            secrets_with(AZURE_DI_ENDPOINT_ENV, "https://env.example"),
        ),
        Object::new(),
    );
    request.connection.api_base = Some("   ".to_owned());
    assert_eq!(
        analyzed_url(&request),
        "https://env.example/documentintelligence/documentModels/prebuilt-layout:analyze?api-version=2024-11-30"
    );
}

#[test]
fn the_endpoint_secret_completes_the_url_when_no_base_is_set() {
    let request = prepared_request(
        connection(
            None,
            None,
            Vec::new(),
            OcrInputSource::Deployment,
            secrets_with(AZURE_DI_ENDPOINT_ENV, " https://env.example "),
        ),
        Object::new(),
    );
    assert_eq!(
        analyzed_url(&request),
        "https://env.example/documentintelligence/documentModels/prebuilt-layout:analyze?api-version=2024-11-30"
    );
}

#[test]
fn a_missing_endpoint_is_the_document_intelligence_authentication_error() {
    let request = prepared_request(
        connection(
            None,
            None,
            Vec::new(),
            OcrInputSource::Deployment,
            Secrets::new(),
        ),
        Object::new(),
    );
    let error = AzureDocIntelligenceOcrConfig
        .get_complete_url(&request, &map_params(&params("{}")).unwrap(), &Vec::new())
        .expect_err("the endpoint is missing");
    let OcrError::Auth(OcrAuthError::MissingApiBase { provider, guidance }) = error else {
        panic!("the missing endpoint is an authentication error");
    };
    assert_eq!(provider, "Azure Document Intelligence");
    assert!(guidance.contains(AZURE_DI_ENDPOINT_ENV));
}

fn settings(poll_timeout: Duration, dpi: i64, api_version: &str) -> OcrSettings {
    OcrSettings {
        request_timeout: Duration::from_mins(100),
        max_download_bytes: 52_428_800,
        poll_timeout,
        doc_intelligence_api_version: api_version.to_owned(),
        doc_intelligence_dpi: dpi,
        vertex_project: None,
        vertex_location: None,
        enable_azure_ad_token_refresh: false,
    }
}

#[test]
fn the_configured_api_version_is_used() {
    let mut request = keyed_request(ENDPOINT, Object::new());
    request.connection.settings = settings(Duration::from_secs(120), 96, "2025-01-01");
    assert_eq!(
        analyzed_url(&request),
        "https://doc-intel.example/documentintelligence/documentModels/prebuilt-layout:analyze?api-version=2025-01-01"
    );
}

#[test]
fn the_config_reports_its_options_and_secrets() {
    let config = AzureDocIntelligenceOcrConfig;
    assert_eq!(
        config.get_supported_ocr_params("model"),
        &["pages", "features", "req_format"]
    );
    assert_eq!(config.get_api_key_env_var(), Some(AZURE_DI_API_KEY_ENV));
    let secrets = config.secret_names();
    assert!(secrets.contains(&AZURE_DI_API_KEY_ENV));
    assert!(secrets.contains(&AZURE_DI_ENDPOINT_ENV));
    assert!(secrets.contains(&"AZURE_CLIENT_SECRET"));
}

struct StaticAzure {
    token: Option<OcrSourced<SecretString>>,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl AzureTokenSource for StaticAzure {
    async fn token(
        &self,
        _request: &PreparedOcrRequest,
    ) -> Result<Option<OcrSourced<SecretString>>, OcrError> {
        let _count = self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.token.clone())
    }
}

fn azure_client(token: Option<(&str, OcrInputSource)>) -> OcrClient {
    let source = StaticAzure {
        token: token.map(|(value, source)| OcrSourced {
            value: SecretString::from(value),
            source,
        }),
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let mut client = plain_client();
    client.azure = Arc::new(source);
    client
}

async fn resolved_headers(
    connection: OcrConnection,
    token: Option<(&str, OcrInputSource)>,
) -> Result<Vec<(String, String)>, OcrError> {
    let request = prepared_request(connection, Object::new());
    AzureDocIntelligenceOcrConfig
        .validate_environment(&request, &azure_client(token))
        .await
}

async fn headers_of(connection: OcrConnection) -> Vec<(String, String)> {
    resolved_headers(connection, None)
        .await
        .expect("the headers resolve")
}

#[tokio::test]
async fn a_supplied_authorization_header_wins_over_every_credential() {
    let headers = headers_of(connection(
        Some(("request-key", OcrInputSource::Request)),
        Some((ENDPOINT, OcrInputSource::Request)),
        vec![("Authorization", "Bearer supplied"), ("x-caller", "yes")],
        OcrInputSource::Request,
        secrets_with(AZURE_DI_API_KEY_ENV, "env-key"),
    ))
    .await;
    assert_eq!(
        headers,
        vec![
            ("Authorization".to_owned(), "Bearer supplied".to_owned()),
            ("x-caller".to_owned(), "yes".to_owned()),
        ]
    );
}

#[tokio::test]
async fn a_supplied_subscription_key_header_wins_over_every_credential() {
    let headers = resolved_headers(
        connection(
            Some(("request-key", OcrInputSource::Request)),
            Some((ENDPOINT, OcrInputSource::Request)),
            vec![(AZURE_DI_SUBSCRIPTION_HEADER, "supplied-key")],
            OcrInputSource::Request,
            Secrets::new(),
        ),
        Some(("token", OcrInputSource::Request)),
    )
    .await
    .unwrap();
    assert_eq!(
        headers,
        vec![(
            AZURE_DI_SUBSCRIPTION_HEADER.to_owned(),
            "supplied-key".to_owned()
        )]
    );
}

#[tokio::test]
async fn the_request_key_becomes_the_subscription_header() {
    let headers = headers_of(connection(
        Some(("request-key", OcrInputSource::Request)),
        Some((ENDPOINT, OcrInputSource::Request)),
        vec![("x-caller", "yes")],
        OcrInputSource::Request,
        Secrets::new(),
    ))
    .await;
    assert_eq!(
        headers,
        vec![
            (
                AZURE_DI_SUBSCRIPTION_HEADER.to_owned(),
                "request-key".to_owned()
            ),
            ("x-caller".to_owned(), "yes".to_owned()),
        ]
    );
}

#[tokio::test]
async fn a_blank_request_key_falls_back_to_the_environment_key() {
    let headers = headers_of(connection(
        Some(("  ", OcrInputSource::Request)),
        Some((ENDPOINT, OcrInputSource::Deployment)),
        Vec::new(),
        OcrInputSource::Deployment,
        secrets_with(AZURE_DI_API_KEY_ENV, "env-key"),
    ))
    .await;
    assert_eq!(
        headers,
        vec![(
            AZURE_DI_SUBSCRIPTION_HEADER.to_owned(),
            "env-key".to_owned()
        )]
    );
}

#[tokio::test]
async fn an_entra_token_becomes_the_authorization_bearer() {
    let headers = resolved_headers(
        connection(
            None,
            Some((ENDPOINT, OcrInputSource::Deployment)),
            Vec::new(),
            OcrInputSource::Deployment,
            Secrets::new(),
        ),
        Some(("entra-token", OcrInputSource::Environment)),
    )
    .await
    .unwrap();
    assert_eq!(
        headers,
        vec![("Authorization".to_owned(), "Bearer entra-token".to_owned())]
    );
}

#[rstest]
#[case::absent(None)]
#[case::blank(Some(("", OcrInputSource::Environment)))]
#[tokio::test]
async fn missing_credentials_are_the_document_intelligence_error(
    #[case] token: Option<(&str, OcrInputSource)>,
) {
    let error = resolved_headers(
        connection(
            None,
            Some((ENDPOINT, OcrInputSource::Deployment)),
            Vec::new(),
            OcrInputSource::Deployment,
            Secrets::new(),
        ),
        token,
    )
    .await
    .expect_err("no credential resolves");
    assert!(matches!(error, OcrError::MissingDocIntelCredentials));
}

#[tokio::test]
async fn a_host_key_is_refused_for_a_request_supplied_endpoint() {
    let error = resolved_headers(
        connection(
            None,
            Some((ENDPOINT, OcrInputSource::Request)),
            Vec::new(),
            OcrInputSource::Deployment,
            secrets_with(AZURE_DI_API_KEY_ENV, "env-key"),
        ),
        None,
    )
    .await
    .expect_err("a host key must not reach a request endpoint");
    assert!(matches!(
        error,
        OcrError::Auth(OcrAuthError::InvalidConfiguration { .. })
    ));
}

#[tokio::test]
async fn a_host_entra_token_is_refused_for_a_request_supplied_endpoint() {
    let error = resolved_headers(
        connection(
            None,
            Some((ENDPOINT, OcrInputSource::Request)),
            Vec::new(),
            OcrInputSource::Deployment,
            Secrets::new(),
        ),
        Some(("entra-token", OcrInputSource::Environment)),
    )
    .await
    .expect_err("a host token must not reach a request endpoint");
    assert!(matches!(
        error,
        OcrError::Auth(OcrAuthError::InvalidConfiguration { .. })
    ));
}

fn request_body(document: OcrDocument) -> Json {
    to_json(
        &AzureDocIntelligenceOcrConfig
            .transform_ocr_request(
                document,
                OcrTransform {
                    model: "model",
                    optional_params: &map_params(&params("{}")).unwrap(),
                    headers: &[],
                },
            )
            .expect("the document becomes a body"),
    )
    .expect("the body is JSON")
}

#[test]
fn a_document_url_becomes_the_url_source() {
    assert_eq!(
        request_body(document("https://example.test/a.pdf")),
        Json::Object(params(r#"{"urlSource":"https://example.test/a.pdf"}"#))
    );
}

#[test]
fn a_data_uri_becomes_the_base64_source() {
    assert_eq!(
        request_body(document("data:application/pdf;base64,YWJj")),
        Json::Object(params(r#"{"base64Source":"YWJj"}"#))
    );
}

#[test]
fn an_empty_document_source_is_the_missing_document_error() {
    let error = AzureDocIntelligenceOcrConfig
        .transform_ocr_request(
            document(""),
            OcrTransform {
                model: "model",
                optional_params: &map_params(&params("{}")).unwrap(),
                headers: &[],
            },
        )
        .expect_err("the source is empty");
    assert!(matches!(error, OcrError::MissingDocumentUrl));
}

fn succeeded(content: &str) -> String {
    format!(
        r#"{{"status":"succeeded","analyzeResult":{{"content":"{content}","pages":[{{"pageNumber":2,"width":8.5,"height":11,"unit":"inch","lines":[{{"content":"first"}},{{"content":"second"}}]}},{{"width":100.9,"height":-2.4,"unit":"pixel","lines":[{{"content":"only"}}]}}],"tables":[{{"rowCount":1}}],"keyValuePairs":[{{"key":{{"content":"k"}}}}]}}}}"#
    )
}

fn normalized(raw: &str) -> Result<OCRResponse, OcrError> {
    AzureDocIntelligenceOcrConfig.transform_ocr_response(
        "doc-intelligence/prebuilt-layout",
        raw.as_bytes(),
        RequestFormat::Litellm,
    )
}

fn first_page(raw: &str) -> OCRPage {
    normalized(raw)
        .expect("the operation normalizes")
        .pages
        .remove(0)
}

#[test]
fn a_finished_operation_becomes_the_ocr_response() {
    let response = normalized(&succeeded("hello")).unwrap();
    assert_eq!(response.model, "doc-intelligence/prebuilt-layout");
    assert_eq!(response.object, "ocr");
    assert_eq!(response.content.as_deref(), Some("hello"));
    assert_eq!(response.tables.as_ref().map(Vec::len), Some(1));
    assert_eq!(response.key_value_pairs.as_ref().map(Vec::len), Some(1));
    assert_eq!(
        response
            .usage_info
            .as_ref()
            .and_then(|usage| usage.pages_processed),
        Some(2)
    );
    assert_eq!(response.pages.len(), 2);
}

#[test]
fn the_pages_carry_markdown_and_pixel_dimensions() {
    let response = normalized(&succeeded("hello")).unwrap();
    let page = &response.pages[0];
    assert_eq!(page.index, 1);
    assert_eq!(page.markdown, "first\nsecond");
    let dimensions = page.dimensions.as_ref().unwrap();
    assert_eq!(dimensions.width, Some(816));
    assert_eq!(dimensions.height, Some(1056));
    assert_eq!(dimensions.dpi, Some(96));
    let second = &response.pages[1];
    assert_eq!(second.index, 0);
    assert_eq!(second.markdown, "only");
    let second_dimensions = second.dimensions.as_ref().unwrap();
    assert_eq!(second_dimensions.width, Some(100));
    assert_eq!(second_dimensions.height, Some(-2));
}

#[test]
fn a_missing_analyze_result_is_an_empty_document() {
    let response = normalized(r#"{"status":"succeeded"}"#).unwrap();
    assert!(response.pages.is_empty());
    assert_eq!(
        response
            .usage_info
            .as_ref()
            .and_then(|usage| usage.pages_processed),
        Some(0)
    );
}

#[rstest]
#[case::failed(r#"{"status":"failed"}"#, "failed")]
#[case::unknown(r#"{"status":"canceled"}"#, "canceled")]
#[case::missing(r#"{"analyzeResult":{}}"#, "None")]
fn a_non_succeeded_operation_is_an_operation_status(#[case] raw: &str, #[case] status: &str) {
    let error = normalized(raw).expect_err("the operation is not finished");
    assert!(matches!(error, OcrError::OperationStatus(text) if text == status));
}

#[test]
fn a_page_zero_is_a_numeric_range_error() {
    let error =
        normalized(r#"{"status":"succeeded","analyzeResult":{"pages":[{"pageNumber":0}]}}"#)
            .expect_err("a page number below one is out of range");
    assert!(matches!(error, OcrError::NumericRange("page.pageNumber")));
}

#[test]
fn a_dimension_at_the_positive_boundary_is_a_numeric_range_error() {
    let error = normalized(
        r#"{"status":"succeeded","analyzeResult":{"pages":[{"unit":"pixel","width":9223372036854775808}]}}"#,
    )
    .expect_err("2^63 is outside the i64 range");
    assert!(matches!(error, OcrError::NumericRange("page.width")));
}

#[test]
fn a_dimension_at_the_negative_boundary_is_the_smallest_i64() {
    let page = first_page(
        r#"{"status":"succeeded","analyzeResult":{"pages":[{"unit":"pixel","width":-9223372036854775808}]}}"#,
    );
    assert_eq!(page.dimensions.unwrap().width, Some(i64::MIN));
}

#[test]
fn an_infinite_scaled_dimension_is_a_numeric_range_error() {
    let error = normalized(r#"{"status":"succeeded","analyzeResult":{"pages":[{"width":1e308}]}}"#)
        .expect_err("the scaled width overflows");
    assert!(matches!(error, OcrError::NumericRange("page.width")));
}

#[test]
fn a_missing_page_number_and_unit_use_the_defaults() {
    let page = first_page(r#"{"status":"succeeded","analyzeResult":{"pages":[{}]}}"#);
    assert_eq!(page.index, 0);
    assert_eq!(page.markdown, "");
    let dimensions = page.dimensions.unwrap();
    assert_eq!(dimensions.width, Some(816));
    assert_eq!(dimensions.height, Some(1056));
    assert_eq!(dimensions.dpi, Some(96));
}

#[rstest]
#[case::whole_float(r#"{"pageNumber":2.0}"#, 1)]
#[case::numeric_string(r#"{"pageNumber":"2"}"#, 1)]
#[case::boolean(r#"{"pageNumber":true}"#, 0)]
fn lax_page_numbers_follow_the_declared_types(#[case] page: &str, #[case] expected: i64) {
    let body = format!(r#"{{"status":"succeeded","analyzeResult":{{"pages":[{page}]}}}}"#);
    assert_eq!(first_page(&body).index, expected, "{page}");
}

#[rstest]
#[case::plus_sign("+2", Some(2))]
#[case::zero_fraction("2.0", Some(2))]
#[case::underscores("1_000", Some(1000))]
#[case::empty("", None)]
#[case::double_underscore("1__0", None)]
#[case::leading_underscore("_1", None)]
#[case::trailing_underscore("1_", None)]
#[case::letters("12a", None)]
#[case::trailing_dot("2.", None)]
#[case::fraction("2.5", None)]
fn page_number_strings_follow_the_declared_type(#[case] page: &str, #[case] expected: Option<i64>) {
    let body = format!(
        r#"{{"status":"succeeded","analyzeResult":{{"pages":[{{"pageNumber":"{page}"}}]}}}}"#
    );
    match expected {
        Some(number) => assert_eq!(
            first_page(&body).index,
            number.checked_sub(1).unwrap(),
            "{page}"
        ),
        None => assert!(
            matches!(normalized(&body), Err(OcrError::ResponseField { .. })),
            "{page}"
        ),
    }
}

#[rstest]
#[case::string_and_boolean(r#"{"unit":"inch","width":" 8.5 ","height":true}"#)]
#[case::numbers(r#"{"unit":"inch","width":8.5,"height":1}"#)]
fn lax_dimensions_follow_the_declared_types(#[case] page: &str) {
    let body = format!(r#"{{"status":"succeeded","analyzeResult":{{"pages":[{page}]}}}}"#);
    let dimensions = first_page(&body).dimensions.unwrap();
    assert_eq!(dimensions.width, Some(816), "{page}");
    assert_eq!(dimensions.height, Some(96), "{page}");
}

#[rstest]
#[case::fractional_page(r#"{"pageNumber":2.5}"#)]
#[case::object_page(r#"{"pageNumber":{"value":2}}"#)]
#[case::text_dimension(r#"{"width":"wide"}"#)]
#[case::infinite_text_dimension(r#"{"width":"inf"}"#)]
fn a_value_the_declared_type_rejects_is_a_response_field(#[case] page: &str) {
    let body = format!(r#"{{"status":"succeeded","analyzeResult":{{"pages":[{page}]}}}}"#);
    let error = normalized(&body).expect_err("the page cannot be decoded");
    assert!(matches!(error, OcrError::ResponseField { .. }), "{error:?}");
}

#[test]
fn the_sync_path_uses_the_default_dpi_not_the_connection() {
    let response = normalized(
        r#"{"status":"succeeded","analyzeResult":{"pages":[{"width":8.5,"height":11}]}}"#,
    )
    .unwrap();
    let dimensions = response.pages[0].dimensions.as_ref().unwrap();
    assert_eq!(dimensions.width, Some(816));
    assert_eq!(dimensions.dpi, Some(96));
}

#[test]
fn native_requests_keep_the_operation_in_the_native_slot() {
    let response = AzureDocIntelligenceOcrConfig
        .transform_ocr_response(
            "model",
            succeeded("hello").as_bytes(),
            RequestFormat::Native,
        )
        .unwrap();
    assert!(matches!(
        response.provider_native_response,
        Some(Json::Object(_))
    ));
}

#[test]
fn a_litellm_request_keeps_no_native_slot() {
    let response = normalized(&succeeded("hello")).unwrap();
    assert!(response.provider_native_response.is_none());
}
