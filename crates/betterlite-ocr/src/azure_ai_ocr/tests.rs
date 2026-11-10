#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::outbound::RequestBody;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::json::{Json, Object};
use crate::mistral_ocr::MistralOcrConfig;
use crate::ocr_auth::AzureTokenSource;
use crate::ocr_config::connection::{
    OcrConnection, OcrInputSource, OcrSourced, OcrTransportConfig, ResolvedOcrCredentials,
};
use crate::ocr_config::{
    AzureAdTokenProvider, OCR_RESPONSE_MAX_BYTES, OcrProviderConfig as _, OcrTransform,
    PreparedOcrRequest, Secrets,
};
use crate::ocr_document::OcrDocument;
use crate::ocr_error::{OcrAuthError, OcrError};
use crate::ocr_http::tests::support::{MockServer, ocr_client, reply};
use crate::ocr_http::{CallHooks, OcrClient, OcrWireRequest, ocr};
use crate::ocr_settings::OcrSettings;
use crate::request_format::RequestFormat;

use super::auth::{resolve_entra, validate_destination};
use super::{AZURE_AI_API_BASE_ENV, AZURE_AI_API_KEY_ENV, AzureAiOcrConfig};

const REFUSAL: &str = "host credentials cannot be sent to a request-controlled Azure endpoint";

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

fn request(
    connection: OcrConnection,
    document: OcrDocument,
    params: Object,
    provider: Option<Arc<dyn AzureAdTokenProvider>>,
) -> PreparedOcrRequest {
    PreparedOcrRequest {
        model: "azure-ai-mistral-ocr".to_owned(),
        document,
        connection,
        caller_document: false,
        optional_params: params,
        input_sources: BTreeMap::new(),
        azure_ad_token_provider: provider,
    }
}

fn inline_request() -> PreparedOcrRequest {
    request(
        connection(
            None,
            Some(("https://example.com", OcrInputSource::Deployment)),
            Vec::new(),
            OcrInputSource::Deployment,
            Secrets::new(),
        ),
        document("data:application/pdf;base64,YWJj"),
        Object::new(),
        None,
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
        None,
    )
}

async fn comparable_bodies() -> (Vec<u8>, Vec<u8>) {
    let inline = document("data:application/pdf;base64,YWJj");
    let options = params(r#"{"pages":[0,2],"extract_header":true,"req_format":"litellm"}"#);
    let azure_request = keyed_request(
        "https://azure.example",
        OcrInputSource::Request,
        inline.clone(),
        options.clone(),
    );
    let mistral_request = keyed_request(
        "https://api.mistral.ai/v1",
        OcrInputSource::Deployment,
        inline,
        options,
    );
    let client = plain_client();
    let azure = AzureAiOcrConfig
        .prepare_request(&azure_request, &client, &NoHooks)
        .await
        .expect("the Azure request prepares");
    let mistral = MistralOcrConfig
        .prepare_request(&mistral_request, &client, &NoHooks)
        .await
        .expect("the direct request prepares");
    let RequestBody::Json(azure_body) = azure.body else {
        panic!("the Azure body is JSON");
    };
    let RequestBody::Json(mistral_body) = mistral.body else {
        panic!("the Mistral body is JSON");
    };
    (azure_body, mistral_body)
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

struct FailingAzure;

#[async_trait]
impl AzureTokenSource for FailingAzure {
    async fn token(
        &self,
        _request: &PreparedOcrRequest,
    ) -> Result<Option<OcrSourced<SecretString>>, OcrError> {
        Err(OcrError::Auth(OcrAuthError::InvalidConfiguration {
            detail: "the Azure token provider refused".to_owned(),
        }))
    }
}

struct UnusedTokenProvider;

#[async_trait]
impl AzureAdTokenProvider for UnusedTokenProvider {
    async fn token(&self) -> Result<SecretString, OcrError> {
        Ok(SecretString::from("unused"))
    }
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

fn client_with(source: Arc<dyn AzureTokenSource>) -> OcrClient {
    let mut client = ocr_client(OcrSettings::default(), Duration::from_secs(5));
    client.azure = source;
    client
}

fn counting_azure(
    token: Option<(&str, OcrInputSource)>,
) -> (Arc<dyn AzureTokenSource>, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let source = StaticAzure {
        token: token.map(|(value, source)| OcrSourced {
            value: SecretString::from(value),
            source,
        }),
        calls: Arc::clone(&calls),
    };
    (Arc::new(source), calls)
}

fn plain_client() -> OcrClient {
    ocr_client(OcrSettings::default(), Duration::from_secs(5))
}

#[rstest]
#[case::bare_base(
    "https://example.com",
    "https://example.com/providers/mistral/azure/ocr"
)]
#[case::query_is_kept(
    "https://example.com/?tenant=a",
    "https://example.com/providers/mistral/azure/ocr?tenant=a"
)]
#[case::complete_endpoint(
    "https://example.com/providers/mistral/azure/ocr",
    "https://example.com/providers/mistral/azure/ocr"
)]
#[case::complete_endpoint_with_query(
    "https://example.com/providers/mistral/azure/ocr?tenant=a",
    "https://example.com/providers/mistral/azure/ocr?tenant=a"
)]
#[case::partial_path(
    "https://example.com/providers/mistral",
    "https://example.com/providers/mistral/azure/ocr"
)]
#[case::unrelated_suffix(
    "https://example.com/mistral/azure",
    "https://example.com/mistral/azure/providers/mistral/azure/ocr"
)]
#[case::base_is_trimmed(
    "  https://example.com  ",
    "https://example.com/providers/mistral/azure/ocr"
)]
fn completes_a_base_to_the_azure_mistral_ocr_url(#[case] api_base: &str, #[case] expected: &str) {
    let request = request(
        connection(
            None,
            Some((api_base, OcrInputSource::Request)),
            Vec::new(),
            OcrInputSource::Deployment,
            Secrets::new(),
        ),
        document("data:application/pdf;base64,YWJj"),
        Object::new(),
        None,
    );
    assert_eq!(
        AzureAiOcrConfig
            .get_complete_url(&request, &Object::new(), &Vec::new())
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
            secrets_with(AZURE_AI_API_BASE_ENV, " https://env.example "),
        ),
        document("data:application/pdf;base64,YWJj"),
        Object::new(),
        None,
    );
    assert_eq!(
        AzureAiOcrConfig
            .get_complete_url(&request, &Object::new(), &Vec::new())
            .expect("the environment base completes"),
        "https://env.example/providers/mistral/azure/ocr"
    );
}

#[test]
fn a_missing_api_base_is_the_azure_ai_missing_base_error() {
    let request = request(
        connection(
            None,
            None,
            Vec::new(),
            OcrInputSource::Deployment,
            Secrets::new(),
        ),
        document("data:application/pdf;base64,YWJj"),
        Object::new(),
        None,
    );
    let error = AzureAiOcrConfig
        .get_complete_url(&request, &Object::new(), &Vec::new())
        .expect_err("a missing base fails");
    let OcrError::Auth(auth) = &error else {
        panic!("the missing base is the Azure AI authentication error");
    };
    let OcrAuthError::MissingApiBase { provider, guidance } = auth else {
        panic!("the missing base names the missing API base");
    };
    assert_eq!(*provider, "Azure AI");
    assert_eq!(
        *guidance,
        "Set AZURE_AI_API_BASE environment variable or pass api_base parameter"
    );
    assert!(auth.to_string().contains(*guidance), "{auth}");
}

#[tokio::test]
async fn a_missing_api_base_precedes_the_supplied_authorization_header() {
    let request = request(
        connection(
            None,
            None,
            vec![("Authorization".to_owned(), "Bearer forwarded".to_owned())],
            OcrInputSource::Deployment,
            Secrets::new(),
        ),
        document("data:application/pdf;base64,YWJj"),
        Object::new(),
        None,
    );
    let error = AzureAiOcrConfig
        .validate_environment(&request, &plain_client())
        .await
        .expect_err("the missing base wins over the header");
    assert!(matches!(
        &error,
        OcrError::Auth(OcrAuthError::MissingApiBase { .. })
    ));
}

#[rstest]
#[case::relative_path("relative/path")]
#[case::cannot_be_a_base("mailto:ops@example.com")]
fn an_invalid_api_base_is_the_api_base_request_field(#[case] api_base: &str) {
    let request = request(
        connection(
            None,
            Some((api_base, OcrInputSource::Request)),
            Vec::new(),
            OcrInputSource::Deployment,
            Secrets::new(),
        ),
        document("data:application/pdf;base64,YWJj"),
        Object::new(),
        None,
    );
    assert!(matches!(
        AzureAiOcrConfig.get_complete_url(&request, &Object::new(), &Vec::new()),
        Err(OcrError::RequestField { path }) if path == "api_base"
    ));
}

#[rstest]
#[case::lowercase("authorization")]
#[case::mixed_case("Authorization")]
#[tokio::test]
async fn a_supplied_authorization_header_wins_over_both_keys(#[case] name: &str) {
    let headers = vec![
        (name.to_owned(), "Bearer forwarded".to_owned()),
        ("X-Trace".to_owned(), "trace-1".to_owned()),
    ];
    let request = request(
        connection(
            Some(("request-key", OcrInputSource::Request)),
            Some(("https://example.com", OcrInputSource::Deployment)),
            headers.clone(),
            OcrInputSource::Deployment,
            secrets_with(AZURE_AI_API_KEY_ENV, "environment-key"),
        ),
        document("data:application/pdf;base64,YWJj"),
        Object::new(),
        None,
    );
    let (azure, calls) = counting_azure(Some(("entra-token", OcrInputSource::Deployment)));
    let environment = AzureAiOcrConfig
        .validate_environment(&request, &client_with(azure))
        .await
        .expect("the supplied header wins");
    assert_eq!(environment, headers);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_configured_token_provider_is_resolved_once_with_a_supplied_header() {
    let headers = vec![("Authorization".to_owned(), "Bearer forwarded".to_owned())];
    let request = request(
        connection(
            None,
            Some(("https://example.com", OcrInputSource::Deployment)),
            headers.clone(),
            OcrInputSource::Deployment,
            Secrets::new(),
        ),
        document("data:application/pdf;base64,YWJj"),
        Object::new(),
        Some(Arc::new(UnusedTokenProvider)),
    );
    let (azure, calls) = counting_azure(Some(("entra-token", OcrInputSource::Deployment)));
    let environment = AzureAiOcrConfig
        .validate_environment(&request, &client_with(azure))
        .await
        .expect("the supplied header wins");
    assert_eq!(environment, headers);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_token_provider_failure_reaches_the_caller_with_a_supplied_header() {
    let request = request(
        connection(
            None,
            Some(("https://example.com", OcrInputSource::Deployment)),
            vec![("Authorization".to_owned(), "Bearer forwarded".to_owned())],
            OcrInputSource::Deployment,
            Secrets::new(),
        ),
        document("data:application/pdf;base64,YWJj"),
        Object::new(),
        Some(Arc::new(UnusedTokenProvider)),
    );
    let error = AzureAiOcrConfig
        .validate_environment(&request, &client_with(Arc::new(FailingAzure)))
        .await
        .expect_err("the provider failure is not swallowed");
    assert!(matches!(
        &error,
        OcrError::Auth(OcrAuthError::InvalidConfiguration { detail })
            if detail == "the Azure token provider refused"
    ));
}

#[rstest]
#[case::request_key(Some("request-key"), Some("env-key"), "Bearer request-key")]
#[case::environment_key(None, Some("env-key"), "Bearer env-key")]
#[case::blank_request_key(Some("   "), Some("env-key"), "Bearer env-key")]
#[case::trimmed_environment_key(None, Some("  env-key  "), "Bearer env-key")]
#[tokio::test]
async fn the_key_order_is_request_then_environment(
    #[case] api_key: Option<&str>,
    #[case] environment_key: Option<&str>,
    #[case] expected: &str,
) {
    let secrets = environment_key.map_or_else(Secrets::new, |value| {
        secrets_with(AZURE_AI_API_KEY_ENV, value)
    });
    let request = request(
        connection(
            api_key.map(|value| (value, OcrInputSource::Request)),
            Some(("https://example.com", OcrInputSource::Deployment)),
            Vec::new(),
            OcrInputSource::Deployment,
            secrets,
        ),
        document("data:application/pdf;base64,YWJj"),
        Object::new(),
        None,
    );
    let environment = AzureAiOcrConfig
        .validate_environment(&request, &plain_client())
        .await
        .expect("a key resolves");
    assert_eq!(
        environment,
        [("Authorization".to_owned(), expected.to_owned())]
    );
}

#[tokio::test]
async fn extra_headers_follow_the_generated_authorization_header() {
    let extra = vec![("X-Trace".to_owned(), "trace-1".to_owned())];
    let request = request(
        connection(
            Some(("request-key", OcrInputSource::Request)),
            Some(("https://example.com", OcrInputSource::Deployment)),
            extra,
            OcrInputSource::Deployment,
            Secrets::new(),
        ),
        document("data:application/pdf;base64,YWJj"),
        Object::new(),
        None,
    );
    let environment = AzureAiOcrConfig
        .validate_environment(&request, &plain_client())
        .await
        .expect("the key resolves");
    assert_eq!(
        environment,
        [
            ("Authorization".to_owned(), "Bearer request-key".to_owned()),
            ("X-Trace".to_owned(), "trace-1".to_owned()),
        ]
    );
}

#[tokio::test]
async fn an_entra_token_becomes_the_bearer_header_without_a_key() {
    let request = request(
        connection(
            None,
            Some(("https://example.com", OcrInputSource::Deployment)),
            Vec::new(),
            OcrInputSource::Deployment,
            Secrets::new(),
        ),
        document("data:application/pdf;base64,YWJj"),
        Object::new(),
        None,
    );
    let (azure, calls) = counting_azure(Some(("entra-token", OcrInputSource::Deployment)));
    let environment = AzureAiOcrConfig
        .validate_environment(&request, &client_with(azure))
        .await
        .expect("the token resolves");
    assert_eq!(
        environment,
        [("Authorization".to_owned(), "Bearer entra-token".to_owned())]
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
#[tokio::test]
async fn no_key_and_no_token_is_the_missing_azure_ai_credentials_error(
    #[case] token: Option<&str>,
) {
    let request = request(
        connection(
            None,
            Some(("https://example.com", OcrInputSource::Deployment)),
            Vec::new(),
            OcrInputSource::Deployment,
            Secrets::new(),
        ),
        document("data:application/pdf;base64,YWJj"),
        Object::new(),
        None,
    );
    let (azure, calls) = counting_azure(token.map(|value| (value, OcrInputSource::Deployment)));
    let error = AzureAiOcrConfig
        .validate_environment(&request, &client_with(azure))
        .await
        .expect_err("a missing credential fails");
    assert!(matches!(&error, OcrError::MissingAzureAiCredentials));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        error.to_string(),
        "missing Azure AI credentials: set AZURE_AI_API_KEY or configure Entra ID"
    );
}

#[rstest]
#[case::environment(OcrInputSource::Environment)]
#[case::deployment(OcrInputSource::Deployment)]
#[tokio::test]
async fn an_entra_token_is_not_sent_to_a_request_controlled_base(#[case] source: OcrInputSource) {
    let request = request(
        connection(
            None,
            Some(("https://request.example", OcrInputSource::Request)),
            Vec::new(),
            OcrInputSource::Deployment,
            Secrets::new(),
        ),
        document("data:application/pdf;base64,YWJj"),
        Object::new(),
        None,
    );
    let (azure, calls) = counting_azure(Some(("entra-token", source)));
    let error = AzureAiOcrConfig
        .validate_environment(&request, &client_with(azure))
        .await
        .expect_err("a host token is refused");
    assert!(matches!(
        &error,
        OcrError::Auth(OcrAuthError::InvalidConfiguration { detail }) if detail == REFUSAL
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
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
        document("data:application/pdf;base64,YWJj"),
        Object::new(),
        None,
    );
    let error = AzureAiOcrConfig
        .validate_environment(&request, &plain_client())
        .await
        .expect_err("a host key is refused");
    assert!(matches!(
        &error,
        OcrError::Auth(OcrAuthError::InvalidConfiguration { detail }) if detail == REFUSAL
    ));
}

#[tokio::test]
async fn a_request_owned_key_is_sent_to_a_request_controlled_base() {
    let request = request(
        connection(
            Some(("request-key", OcrInputSource::Request)),
            Some(("https://request.example", OcrInputSource::Request)),
            Vec::new(),
            OcrInputSource::Deployment,
            Secrets::new(),
        ),
        document("data:application/pdf;base64,YWJj"),
        Object::new(),
        None,
    );
    let environment = AzureAiOcrConfig
        .validate_environment(&request, &plain_client())
        .await
        .expect("the request key may go to the request base");
    assert_eq!(
        environment,
        [("Authorization".to_owned(), "Bearer request-key".to_owned())]
    );
}

#[rstest]
#[case::request_header(OcrInputSource::Request)]
#[case::host_header(OcrInputSource::Deployment)]
#[tokio::test]
async fn a_supplied_authorization_header_follows_the_destination_rule(
    #[case] header_source: OcrInputSource,
) {
    let headers = vec![("Authorization".to_owned(), "Bearer forwarded".to_owned())];
    let request = request(
        connection(
            None,
            Some(("https://request.example", OcrInputSource::Request)),
            headers.clone(),
            header_source,
            Secrets::new(),
        ),
        document("data:application/pdf;base64,YWJj"),
        Object::new(),
        None,
    );
    let answer = AzureAiOcrConfig
        .validate_environment(&request, &plain_client())
        .await;
    match header_source {
        OcrInputSource::Request => {
            assert_eq!(answer.expect("the request header is allowed"), headers);
        }
        OcrInputSource::Environment | OcrInputSource::Deployment => assert!(matches!(
            answer,
            Err(OcrError::Auth(OcrAuthError::InvalidConfiguration { detail })) if detail == REFUSAL
        )),
    }
}

#[test]
fn validate_destination_allows_a_host_credential_for_a_deployment_base() {
    let connection = connection(
        None,
        Some(("https://deploy.example", OcrInputSource::Deployment)),
        Vec::new(),
        OcrInputSource::Deployment,
        Secrets::new(),
    );
    validate_destination(&connection, OcrInputSource::Environment)
        .expect("the host credential may go to a host base");
}

#[test]
fn validate_destination_allows_every_credential_without_an_api_base() {
    let connection = connection(
        None,
        None,
        Vec::new(),
        OcrInputSource::Deployment,
        Secrets::new(),
    );
    validate_destination(&connection, OcrInputSource::Deployment)
        .expect("a missing base cannot be request controlled");
}

#[test]
fn validate_destination_ignores_a_request_source_without_an_api_base() {
    let mut connection = connection(
        None,
        Some(("https://request.example", OcrInputSource::Request)),
        Vec::new(),
        OcrInputSource::Deployment,
        Secrets::new(),
    );
    connection.api_base = None;
    validate_destination(&connection, OcrInputSource::Environment)
        .expect("there is no request base to protect");
}

#[test]
fn validate_destination_refuses_a_host_credential_for_a_request_base() {
    let connection = connection(
        None,
        Some(("https://request.example", OcrInputSource::Request)),
        Vec::new(),
        OcrInputSource::Deployment,
        Secrets::new(),
    );
    let error = validate_destination(&connection, OcrInputSource::Environment)
        .expect_err("the host credential is refused");
    assert!(matches!(
        error,
        OcrError::Auth(OcrAuthError::InvalidConfiguration { detail }) if detail == REFUSAL
    ));
}

#[test]
fn validate_destination_allows_a_request_credential_for_a_request_base() {
    let connection = connection(
        None,
        Some(("https://request.example", OcrInputSource::Request)),
        Vec::new(),
        OcrInputSource::Deployment,
        Secrets::new(),
    );
    validate_destination(&connection, OcrInputSource::Request)
        .expect("the request credential belongs to the request base");
}

#[tokio::test]
async fn resolve_entra_returns_the_source_token() {
    let (azure, calls) = counting_azure(Some(("entra-token", OcrInputSource::Environment)));
    let resolved = resolve_entra(&client_with(azure), &inline_request())
        .await
        .expect("the source answers")
        .expect("a token is present");
    assert_eq!(resolved.value.expose_secret(), "entra-token");
    assert_eq!(resolved.source, OcrInputSource::Environment);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn resolve_entra_returns_none_when_the_source_has_no_token() {
    let (azure, _calls) = counting_azure(None);
    let resolved = resolve_entra(&client_with(azure), &inline_request())
        .await
        .expect("the source answers");
    assert!(resolved.is_none());
}

#[tokio::test]
async fn downloads_a_remote_document_and_sends_it_inline() {
    let answer = br#"{"pages":[{"index":0,"markdown":"hello"}],"model":"served-model"}"#;
    let server = MockServer::start(vec![
        reply("200 OK", &[("Content-Type", "application/pdf")], b"abc"),
        reply("200 OK", &[], answer),
    ])
    .await;
    let base = server.base.clone();
    let request = keyed_request(
        &base,
        OcrInputSource::Request,
        document(&format!("{base}/scan.pdf")),
        params(r#"{"pages":[0]}"#),
    );
    let response = ocr(&AzureAiOcrConfig, &plain_client(), &request, &NoHooks)
        .await
        .expect("the call succeeds");
    assert_eq!(response.model, "served-model");
    assert_eq!(response.pages[0].markdown, "hello");
    let requests = server.requests().await;
    assert!(
        requests[0].starts_with("GET /scan.pdf HTTP/1.1"),
        "{}",
        requests[0]
    );
    let sent = &requests[1];
    assert!(
        sent.starts_with("POST /providers/mistral/azure/ocr HTTP/1.1"),
        "{sent}"
    );
    assert!(
        sent.to_ascii_lowercase()
            .contains("authorization: bearer request-key"),
        "{sent}"
    );
    assert!(sent.contains("\"document\":{\"type\":\"document_url\",\"document_url\":\"data:application/pdf;base64,YWJj\"}"), "{sent}");
    assert!(sent.contains("\"pages\":[0]"), "{sent}");
    assert!(!sent.contains(&base), "{sent}");
}

#[tokio::test]
async fn the_mistral_body_matches_the_direct_provider() {
    let (azure_body, mistral_body) = comparable_bodies().await;
    assert_eq!(azure_body, mistral_body);
}

#[test]
fn the_mistral_response_matches_the_direct_provider() {
    let answer = br#"{"pages":[{"index":0,"markdown":"hello"}],"model":"served-model","usage_info":{"pages_processed":1}}"#;
    assert_eq!(
        AzureAiOcrConfig
            .transform_ocr_response("azure-ai-mistral-ocr", answer, RequestFormat::Litellm)
            .expect("the Azure response decodes"),
        MistralOcrConfig
            .transform_ocr_response("azure-ai-mistral-ocr", answer, RequestFormat::Litellm)
            .expect("the Mistral response decodes")
    );
}

#[test]
fn transform_ocr_request_builds_the_mistral_body() {
    let options = params(r#"{"pages":[0]}"#);
    let body = AzureAiOcrConfig
        .transform_ocr_request(
            document("data:application/pdf;base64,YWJj"),
            OcrTransform {
                model: "served-model",
                optional_params: &options,
                headers: &[],
            },
        )
        .expect("the body builds");
    assert_eq!(
        serde_json::to_string(&body).expect("the body is JSON"),
        r#"{"model":"served-model","document":{"type":"document_url","document_url":"data:application/pdf;base64,YWJj"},"pages":[0]}"#
    );
}

#[test]
fn an_inline_document_passes_the_body_validation() {
    let body = params(
        r#"{"model":"m","document":{"type":"document_url","document_url":"data:application/pdf;base64,YWJj"}}"#,
    );
    AzureAiOcrConfig
        .validate_request_body(&Json::Object(body))
        .expect("an inline document passes");
}

#[test]
fn a_remote_document_fails_the_body_validation() {
    let body = params(
        r#"{"model":"m","document":{"type":"document_url","document_url":"https://example.com/a.pdf"}}"#,
    );
    assert!(matches!(
        AzureAiOcrConfig.validate_request_body(&Json::Object(body)),
        Err(OcrError::InvalidDataUri)
    ));
}

#[test]
fn a_body_without_a_document_names_the_field() {
    let body = params(r#"{"model":"m"}"#);
    assert!(matches!(
        AzureAiOcrConfig.validate_request_body(&Json::Object(body)),
        Err(OcrError::RequestField { path }) if path.starts_with("body.document")
    ));
}

#[test]
fn map_ocr_params_keeps_the_mistral_options_only() {
    let input =
        params(r#"{"pages":[0],"extract_header":true,"output_format":"markdown","unknown":true}"#);
    let mapped = AzureAiOcrConfig
        .map_ocr_params(&input, "azure-ai-mistral")
        .expect("the options map");
    assert_eq!(mapped, params(r#"{"pages":[0],"extract_header":true}"#));
}

#[test]
fn the_provider_metadata_matches_the_azure_ai_config() {
    assert_eq!(
        AzureAiOcrConfig.get_api_key_env_var(),
        Some(AZURE_AI_API_KEY_ENV)
    );
    assert_eq!(
        AzureAiOcrConfig.secret_names(),
        [
            AZURE_AI_API_KEY_ENV,
            AZURE_AI_API_BASE_ENV,
            "AZURE_AD_TOKEN",
            "AZURE_TENANT_ID",
            "AZURE_CLIENT_ID",
            "AZURE_CLIENT_SECRET",
            "AZURE_SCOPE",
            "AZURE_AUTHORITY_HOST",
            "AZURE_CREDENTIAL",
            "AZURE_FEDERATED_TOKEN_FILE",
        ]
    );
    assert_eq!(
        AzureAiOcrConfig.get_supported_ocr_params("azure-ai-mistral"),
        MistralOcrConfig.get_supported_ocr_params("azure-ai-mistral")
    );
}
