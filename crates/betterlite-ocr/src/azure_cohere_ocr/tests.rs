#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::outbound::RequestBody;
use rstest::rstest;
use secrecy::SecretString;

use crate::azure_ai_ocr::{AZURE_AI_API_BASE_ENV, AZURE_AI_API_KEY_ENV, AzureAiOcrConfig};
use crate::cohere_ocr::CohereParseConfig;
use crate::cohere_ocr::options::{CohereOptions, OutputFormat};
use crate::json::{Json, Object};
use crate::ocr_config::connection::{
    OcrConnection, OcrInputSource, OcrSourced, OcrTransportConfig, ResolvedOcrCredentials,
};
use crate::ocr_config::{
    OCR_RESPONSE_MAX_BYTES, OcrProviderConfig as _, OcrRequestContext, OcrTransform,
    PreparedOcrRequest, Secrets,
};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::{OcrAuthError, OcrError};
use crate::ocr_http::tests::support::{MockServer, ocr_client, reply};
use crate::ocr_http::{CallHooks, OcrClient, OcrWireRequest, ocr};
use crate::ocr_settings::OcrSettings;
use crate::request_format::RequestFormat;

use super::AzureAICohereParseConfig;

fn image(source: &str) -> OcrDocument {
    OcrDocument::ImageUrl {
        image_url: source.to_owned(),
        extra: BTreeMap::new(),
    }
}

fn document(source: &str) -> OcrDocument {
    OcrDocument::DocumentUrl {
        document_url: source.to_owned(),
        extra: BTreeMap::new(),
    }
}

fn params(raw: &str) -> Object {
    serde_json::from_str(raw).expect("the test options are an object")
}

fn secrets_with(name: &str, value: &str) -> Secrets {
    Secrets::from([(name.to_owned(), SecretString::from(value))])
}

fn connection(
    api_key: Option<(&str, OcrInputSource)>,
    api_base: Option<(&str, OcrInputSource)>,
    extra_headers: Vec<(String, String)>,
    extra_headers_source: OcrInputSource,
    secrets: Secrets,
) -> OcrConnection {
    OcrConnection::new(
        ResolvedOcrCredentials {
            api_key: api_key.map(|(value, source)| OcrSourced {
                value: SecretString::from(value),
                source,
            }),
            api_base: api_base.map(|(value, source)| OcrSourced {
                value: value.to_owned(),
                source,
            }),
        },
        OcrTransportConfig {
            extra_headers,
            extra_headers_source,
            timeout: None,
            max_response_bytes: OCR_RESPONSE_MAX_BYTES,
        },
        OcrSettings::default(),
        secrets,
    )
}

fn request(connection: OcrConnection, document: OcrDocument, params: Object) -> PreparedOcrRequest {
    PreparedOcrRequest {
        model: "cohere-parse".to_owned(),
        document,
        connection,
        caller_document: false,
        optional_params: params,
        input_sources: BTreeMap::new(),
        azure_ad_token_provider: None,
    }
}

fn plain_client() -> OcrClient {
    ocr_client(OcrSettings::default(), Duration::from_secs(5))
}

fn inline_request(base: Option<(&str, OcrInputSource)>) -> PreparedOcrRequest {
    request(
        connection(
            None,
            base,
            Vec::new(),
            OcrInputSource::Deployment,
            Secrets::new(),
        ),
        image("data:image/png;base64,YWJj"),
        Object::new(),
    )
}

fn keyed_request(
    api_base: &str,
    api_base_source: OcrInputSource,
    document: OcrDocument,
    params: Object,
) -> PreparedOcrRequest {
    request(
        connection(
            Some(("request-key", OcrInputSource::Request)),
            Some((api_base, api_base_source)),
            Vec::new(),
            OcrInputSource::Deployment,
            Secrets::new(),
        ),
        document,
        params,
    )
}

async fn comparable_bodies() -> (Vec<u8>, Vec<u8>) {
    let inline = image("data:image/png;base64,YWJj");
    let options = params(r#"{"output_format":"blocks","req_format":"litellm"}"#);
    let azure_request = keyed_request(
        "https://azure.example",
        OcrInputSource::Request,
        inline.clone(),
        options.clone(),
    );
    let cohere_request = keyed_request(
        "https://api.cohere.com",
        OcrInputSource::Deployment,
        inline,
        options,
    );
    let client = plain_client();
    let azure = AzureAICohereParseConfig
        .prepare_request(&azure_request, &client, &NoHooks)
        .await
        .expect("the Azure request prepares");
    let cohere = CohereParseConfig
        .prepare_request(&cohere_request, &client, &NoHooks)
        .await
        .expect("the direct request prepares");
    let RequestBody::Json(azure_body) = azure.body else {
        panic!("the Azure body is JSON");
    };
    let RequestBody::Json(cohere_body) = cohere.body else {
        panic!("the Cohere body is JSON");
    };
    (azure_body, cohere_body)
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

#[rstest]
#[case::bare("", "https://example.com/providers/cohere/v2/parse?tenant=a")]
#[case::models("/models", "https://example.com/providers/cohere/v2/parse?tenant=a")]
#[case::provider_prefix(
    "/providers/cohere/v2",
    "https://example.com/providers/cohere/v2/parse?tenant=a"
)]
#[case::complete_path(
    "/providers/cohere/v2/parse",
    "https://example.com/providers/cohere/v2/parse?tenant=a"
)]
#[case::versioned_path("/v2/parse", "https://example.com/v2/parse?tenant=a")]
#[case::complete_path_with_slash(
    "/providers/cohere/v2/parse/",
    "https://example.com/providers/cohere/v2/parse?tenant=a"
)]
#[case::models_with_slash("/models/", "https://example.com/providers/cohere/v2/parse?tenant=a")]
fn completes_the_parse_url_without_duplicate_paths(#[case] suffix: &str, #[case] expected: &str) {
    let base = format!("https://example.com{suffix}?tenant=a");
    let request = inline_request(Some((&base, OcrInputSource::Request)));
    assert_eq!(
        AzureAICohereParseConfig
            .get_complete_url(&request, &CohereOptions::default(), &Vec::new())
            .expect("the base completes"),
        expected
    );
}

#[test]
fn an_environment_base_completes_the_url() {
    let request = request(
        connection(
            None,
            None,
            Vec::new(),
            OcrInputSource::Deployment,
            secrets_with(AZURE_AI_API_BASE_ENV, "https://env.example/models"),
        ),
        image("data:image/png;base64,YWJj"),
        Object::new(),
    );
    assert_eq!(
        AzureAICohereParseConfig
            .get_complete_url(&request, &CohereOptions::default(), &Vec::new())
            .expect("the environment base completes"),
        "https://env.example/providers/cohere/v2/parse"
    );
}

#[rstest]
#[case::absent(None)]
#[case::blank(Some("   "))]
#[test]
fn a_missing_api_base_is_the_azure_ai_missing_base_error(#[case] api_base: Option<&str>) {
    let request = inline_request(api_base.map(|base| (base, OcrInputSource::Request)));
    let error = AzureAICohereParseConfig
        .get_complete_url(&request, &CohereOptions::default(), &Vec::new())
        .expect_err("a missing base fails");
    let OcrError::Auth(OcrAuthError::MissingApiBase { provider, guidance }) = &error else {
        panic!("the missing base is the Azure AI authentication error");
    };
    assert_eq!(*provider, "Azure AI");
    assert_eq!(
        *guidance,
        "Set AZURE_AI_API_BASE environment variable or pass api_base parameter"
    );
}

#[rstest]
#[case::relative_path("relative/path")]
#[case::unsupported_scheme("ftp://example.com")]
#[case::cannot_be_a_base("mailto:ops@example.com")]
fn a_non_http_base_is_the_api_base_request_field(#[case] api_base: &str) {
    let request = inline_request(Some((api_base, OcrInputSource::Request)));
    assert!(matches!(
        AzureAICohereParseConfig.get_complete_url(&request, &CohereOptions::default(), &Vec::new()),
        Err(OcrError::RequestField { path }) if path == "api_base"
    ));
}

#[tokio::test]
async fn the_request_key_becomes_the_bearer_header() {
    let request = request(
        connection(
            Some(("request-key", OcrInputSource::Request)),
            Some(("https://example.com", OcrInputSource::Deployment)),
            Vec::new(),
            OcrInputSource::Deployment,
            secrets_with(AZURE_AI_API_KEY_ENV, "environment-key"),
        ),
        image("data:image/png;base64,YWJj"),
        Object::new(),
    );
    let environment = AzureAICohereParseConfig
        .validate_environment(&request, &plain_client())
        .await
        .expect("the request key wins");
    assert_eq!(
        environment,
        [("Authorization".to_owned(), "Bearer request-key".to_owned())]
    );
}

#[tokio::test]
async fn a_supplied_authorization_header_wins() {
    let headers = vec![("Authorization".to_owned(), "Bearer forwarded".to_owned())];
    let request = request(
        connection(
            None,
            Some(("https://example.com", OcrInputSource::Deployment)),
            headers.clone(),
            OcrInputSource::Deployment,
            secrets_with(AZURE_AI_API_KEY_ENV, "environment-key"),
        ),
        image("data:image/png;base64,YWJj"),
        Object::new(),
    );
    let environment = AzureAICohereParseConfig
        .validate_environment(&request, &plain_client())
        .await
        .expect("the supplied header wins");
    assert_eq!(environment, headers);
}

#[tokio::test]
async fn a_missing_credential_is_the_missing_azure_ai_credentials_error() {
    let request = request(
        connection(
            None,
            Some(("https://example.com", OcrInputSource::Deployment)),
            Vec::new(),
            OcrInputSource::Deployment,
            Secrets::new(),
        ),
        image("data:image/png;base64,YWJj"),
        Object::new(),
    );
    let error = AzureAICohereParseConfig
        .validate_environment(&request, &plain_client())
        .await
        .expect_err("a missing credential fails");
    assert!(matches!(&error, OcrError::MissingAzureAiCredentials));
}

#[tokio::test]
async fn an_environment_key_is_not_sent_to_a_request_controlled_base() {
    let request = request(
        connection(
            None,
            Some(("https://request.example", OcrInputSource::Request)),
            Vec::new(),
            OcrInputSource::Deployment,
            secrets_with(AZURE_AI_API_KEY_ENV, "environment-key"),
        ),
        image("data:image/png;base64,YWJj"),
        Object::new(),
    );
    let error = AzureAICohereParseConfig
        .validate_environment(&request, &plain_client())
        .await
        .expect_err("a host key is refused");
    assert!(matches!(
        &error,
        OcrError::Auth(OcrAuthError::InvalidConfiguration { detail })
            if detail == "host credentials cannot be sent to a request-controlled Azure endpoint"
    ));
}

#[tokio::test]
async fn downloads_a_remote_image_and_sends_it_inline() {
    let answer = br#"{"pages":[{"index":0,"markdown":{"content":"hello"}}],"meta":{"billed_units":{"pages":1}}}"#;
    let server = MockServer::start(vec![
        reply("200 OK", &[("Content-Type", "image/png")], b"PNG"),
        reply("200 OK", &[], answer),
    ])
    .await;
    let base = server.base.clone();
    let request = keyed_request(
        &base,
        OcrInputSource::Request,
        image(&format!("{base}/scan.png")),
        Object::new(),
    );
    let client = plain_client();
    let response = ocr(&AzureAICohereParseConfig, &client, &request, &NoHooks)
        .await
        .expect("the call succeeds");
    assert_eq!(response.model, "cohere-parse");
    assert_eq!(response.pages[0].markdown, "hello");
    let requests = server.requests().await;
    assert!(
        requests[0].starts_with("GET /scan.png HTTP/1.1"),
        "{}",
        requests[0]
    );
    let sent = &requests[1];
    assert!(
        sent.starts_with("POST /providers/cohere/v2/parse HTTP/1.1"),
        "{sent}"
    );
    assert!(
        sent.contains(
            "\"document\":{\"type\":\"image_url\",\"image_url\":\"data:image/png;base64,UE5H\"}"
        ),
        "{sent}"
    );
    assert!(sent.contains("\"output_format\":\"markdown\""), "{sent}");
    assert!(!sent.contains(&base), "{sent}");
}

#[tokio::test]
async fn the_cohere_body_matches_the_direct_parse_provider() {
    let (azure_body, cohere_body) = comparable_bodies().await;
    assert_eq!(azure_body, cohere_body);
}

#[test]
fn the_cohere_response_matches_the_direct_parse_provider() {
    let answer = br#"{"pages":[{"index":0,"markdown":{"content":"hello"}}],"meta":{"billed_units":{"pages":1}}}"#;
    assert_eq!(
        AzureAICohereParseConfig
            .transform_ocr_response("cohere-parse", answer, RequestFormat::Litellm)
            .expect("the Azure response decodes"),
        CohereParseConfig
            .transform_ocr_response("cohere-parse", answer, RequestFormat::Litellm)
            .expect("the Cohere response decodes")
    );
}

#[rstest]
#[case::document_url(document("https://example.com/a.pdf"))]
#[case::pdf_data_uri(image("data:application/pdf;base64,YWJj"))]
#[tokio::test]
async fn cohere_parse_on_azure_accepts_only_image_documents(#[case] input: OcrDocument) {
    let connection = connection(
        None,
        Some(("https://example.com", OcrInputSource::Deployment)),
        Vec::new(),
        OcrInputSource::Deployment,
        Secrets::new(),
    );
    let options = CohereOptions::default();
    let client = plain_client();
    let error = AzureAICohereParseConfig
        .async_transform_ocr_request(
            input,
            OcrTransform {
                model: "cohere-parse",
                optional_params: &options,
                headers: &[],
            },
            OcrRequestContext {
                client: &client,
                connection: &connection,
            },
        )
        .await
        .expect_err("a non-image document is refused");
    assert!(matches!(error, OcrError::CohereImageOnly));
}

#[test]
fn an_inline_image_passes_the_body_validation() {
    let body = params(
        r#"{"model":"parse","document":{"type":"image_url","image_url":"data:image/png;base64,YWJj"}}"#,
    );
    AzureAICohereParseConfig
        .validate_request_body(&Json::Object(body))
        .expect("an inline image passes");
}

#[test]
fn a_document_url_fails_the_body_validation_as_image_only() {
    let body = params(
        r#"{"model":"parse","document":{"type":"document_url","document_url":"data:image/png;base64,YWJj"}}"#,
    );
    assert!(matches!(
        AzureAICohereParseConfig.validate_request_body(&Json::Object(body)),
        Err(OcrError::CohereImageOnly)
    ));
}

#[test]
fn a_remote_image_fails_the_body_validation_as_not_inline() {
    let body = params(
        r#"{"model":"parse","document":{"type":"image_url","image_url":"https://example.com/a.png"}}"#,
    );
    assert!(matches!(
        AzureAICohereParseConfig.validate_request_body(&Json::Object(body)),
        Err(OcrError::InvalidDataUri)
    ));
}

#[test]
fn a_body_without_a_document_names_the_field() {
    let body = params(r#"{"model":"parse"}"#);
    assert!(matches!(
        AzureAICohereParseConfig.validate_request_body(&Json::Object(body)),
        Err(OcrError::RequestField { path }) if path.starts_with("body.document")
    ));
}

#[test]
fn map_ocr_params_uses_the_cohere_options() {
    let input = params(r#"{"output_format":"blocks","req_format":"native","extension":false}"#);
    let options = AzureAICohereParseConfig
        .map_ocr_params(&input, "cohere-parse")
        .expect("the options map");
    assert_eq!(
        options,
        CohereParseConfig
            .map_ocr_params(&input, "cohere-parse")
            .expect("the direct options map")
    );
    assert_eq!(options.output_format, Some(OutputFormat::Blocks));
}

#[test]
fn the_provider_metadata_matches_the_azure_cohere_config() {
    assert_eq!(
        AzureAICohereParseConfig.get_api_key_env_var(),
        Some(AZURE_AI_API_KEY_ENV)
    );
    assert_eq!(
        AzureAICohereParseConfig.secret_names(),
        AzureAiOcrConfig.secret_names()
    );
    assert_eq!(
        AzureAICohereParseConfig.get_supported_ocr_params("cohere-parse"),
        &["output_format", "req_format"]
    );
    assert_eq!(
        AzureAICohereParseConfig.get_health_check_document(),
        CohereParseConfig.get_health_check_document()
    );
}
