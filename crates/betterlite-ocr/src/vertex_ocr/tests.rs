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
use crate::mistral_ocr::MistralOcrConfig;
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

use super::common;
use super::url::raw_predict_url;
use super::{VertexAiOcrConfig, VertexOcrEnvironment};

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

fn secrets(entries: &[(&str, &str)]) -> Secrets {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), SecretString::from(*value)))
        .collect()
}

fn connection(
    api_base: Option<(&str, OcrInputSource)>,
    api_key: Option<&str>,
    extra_headers: Vec<(String, String)>,
    secrets: Secrets,
    settings: OcrSettings,
) -> OcrConnection {
    OcrConnection::new(
        ResolvedOcrCredentials {
            api_key: api_key.map(|key| OcrSourced {
                value: SecretString::from(key),
                source: OcrInputSource::Request,
            }),
            api_base: api_base.map(|(base, source)| OcrSourced {
                value: base.to_owned(),
                source,
            }),
        },
        OcrTransportConfig {
            extra_headers,
            extra_headers_source: OcrInputSource::Deployment,
            timeout: None,
            max_response_bytes: OCR_RESPONSE_MAX_BYTES,
        },
        settings,
        secrets,
    )
}

fn prepared(connection: OcrConnection, optional_params: Object) -> PreparedOcrRequest {
    PreparedOcrRequest {
        model: "mistral-ocr-latest".to_owned(),
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

fn vertex_request(
    optional_params: Object,
    settings: OcrSettings,
    entries: Secrets,
) -> PreparedOcrRequest {
    prepared(
        connection(None, None, Vec::new(), entries, settings),
        optional_params,
    )
}

fn params(text: &str) -> Object {
    serde_json::from_str(text).expect("the test options are an object")
}

fn document() -> OcrDocument {
    OcrDocument::DocumentUrl {
        document_url: "data:application/pdf;base64,YWJj".to_owned(),
        extra: BTreeMap::new(),
    }
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

#[rstest]
#[case::default_location(
    None,
    "proj-1",
    "us-central1",
    "mistral-ocr-latest",
    "https://us-central1-aiplatform.googleapis.com/v1/projects/proj-1/locations/us-central1/publishers/mistralai/models/mistral-ocr-latest:rawPredict"
)]
#[case::custom_location(
    None,
    "p",
    "europe-west4",
    "m",
    "https://europe-west4-aiplatform.googleapis.com/v1/projects/p/locations/europe-west4/publishers/mistralai/models/m:rawPredict"
)]
#[case::deployment_base(
    Some("https://proxy.example"),
    "p",
    "us-central1",
    "m",
    "https://proxy.example/v1/projects/p/locations/us-central1/publishers/mistralai/models/m:rawPredict"
)]
#[case::base_with_the_first_segment(
    Some("https://proxy.example/v1"),
    "p",
    "us-central1",
    "m",
    "https://proxy.example/v1/projects/p/locations/us-central1/publishers/mistralai/models/m:rawPredict"
)]
#[case::base_query_is_kept(
    Some("https://proxy.example?tenant=a"),
    "p",
    "us-central1",
    "m",
    "https://proxy.example/v1/projects/p/locations/us-central1/publishers/mistralai/models/m:rawPredict?tenant=a"
)]
#[case::base_is_trimmed(
    Some("  https://proxy.example  "),
    "p",
    "us-central1",
    "m",
    "https://proxy.example/v1/projects/p/locations/us-central1/publishers/mistralai/models/m:rawPredict"
)]
#[case::blank_base_uses_the_default(
    Some("   "),
    "p",
    "us-central1",
    "m",
    "https://us-central1-aiplatform.googleapis.com/v1/projects/p/locations/us-central1/publishers/mistralai/models/m:rawPredict"
)]
fn builds_the_vertex_mistral_raw_predict_url(
    #[case] api_base: Option<&str>,
    #[case] project: &str,
    #[case] location: &str,
    #[case] model: &str,
    #[case] expected: &str,
) {
    assert_eq!(
        raw_predict_url(api_base, project, location, model).expect("the URL builds"),
        expected
    );
}

#[rstest]
#[case::empty("")]
#[case::uppercase("US")]
#[case::underscore("us_central1")]
#[case::host_escape("attacker.example/path")]
#[case::leading_hyphen("-us")]
#[case::trailing_hyphen("us-")]
#[case::space("us central1")]
#[case::newline("us\n")]
fn rejects_a_location_that_is_not_a_region_name(#[case] location: &str) {
    let error = raw_predict_url(None, "proj", location, "model")
        .expect_err("the location is not a plain region name");
    assert!(
        matches!(&error, OcrError::RequestField { path } if path == "vertex_location"),
        "{error:?}"
    );
}

#[rstest]
#[case::two_letter("us")]
#[case::global("global")]
#[case::region("europe-west4")]
#[case::numeric_start("1us")]
fn accepts_a_plain_region_name(#[case] location: &str) {
    let url = raw_predict_url(None, "proj", location, "model").expect("the location is a region");
    assert!(
        url.starts_with(&format!(
            "https://{location}-aiplatform.googleapis.com/v1/projects/proj/locations/{location}/publishers/mistralai/models/model:rawPredict"
        )),
        "{url}"
    );
}

#[rstest]
#[case::not_a_url("not a url")]
#[case::cannot_be_a_base("mailto:ops@example.com")]
fn an_api_base_that_does_not_parse_is_the_api_base_request_field(#[case] api_base: &str) {
    let error = raw_predict_url(Some(api_base), "proj", "us-central1", "model")
        .expect_err("the base does not parse");
    assert!(
        matches!(&error, OcrError::RequestField { path } if path == "api_base"),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_request_supplied_api_base_is_refused_before_a_token_or_request() {
    let server = MockServer::start(Vec::new()).await;
    let (source, calls) = TokenSource::fake("ya29.token", Some("proj-1"));
    let connection = connection(
        Some((&server.base, OcrInputSource::Request)),
        None,
        Vec::new(),
        Secrets::new(),
        settings(None, None),
    );
    let error = ocr(
        &VertexAiOcrConfig,
        &client(&source),
        &prepared(connection, Object::new()),
        &NoHooks,
    )
    .await
    .expect_err("a request api_base is refused");
    assert!(
        matches!(
            &error,
            OcrError::Auth(OcrAuthError::InvalidConfiguration { .. })
        ),
        "{error:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(recorded(server).await.is_empty());
}

fn assert_mistral_request(sent: &str) {
    assert!(
        sent.starts_with(
            "POST /v1/projects/proj-9/locations/us-central1/publishers/mistralai/models/mistral-ocr-latest:rawPredict HTTP/1.1"
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
    let expected_document: Json = serde_json::from_str(
        r#"{"type":"document_url","document_url":"data:application/pdf;base64,YWJj"}"#,
    )
    .expect("the document is JSON");
    let expected_pages: Json = serde_json::from_str("[0,2]").expect("the pages are JSON");
    assert_eq!(
        body.get("model"),
        Some(&Json::Text("mistral-ocr-latest".to_owned()))
    );
    assert_eq!(body.get("document"), Some(&expected_document));
    assert_eq!(body.get("pages"), Some(&expected_pages));
    assert_eq!(body.get("extract_header"), Some(&Json::Bool(true)));
}

#[tokio::test]
async fn sends_the_mistral_body_to_the_vertex_raw_predict_endpoint() {
    let answer = br#"{"pages":[{"index":0,"markdown":"hello"}],"model":"served-model"}"#;
    let server = MockServer::start(vec![reply("200 OK", &[], answer)]).await;
    let (source, _calls) = TokenSource::fake("ya29.token", None);
    let connection = connection(
        Some((&server.base, OcrInputSource::Deployment)),
        None,
        Vec::new(),
        Secrets::new(),
        settings(Some("proj-9"), None),
    );
    let response = ocr(
        &VertexAiOcrConfig,
        &client(&source),
        &prepared(
            connection,
            params(r#"{"pages":[0,2],"extract_header":true}"#),
        ),
        &NoHooks,
    )
    .await
    .expect("the call succeeds");
    assert_eq!(response.model, "served-model");
    let requests = recorded(server).await;
    assert_eq!(requests.len(), 1);
    assert_mistral_request(&requests[0]);
}

#[tokio::test]
async fn downloads_a_remote_document_before_the_body_is_built() {
    let file = MockServer::start(vec![reply(
        "200 OK",
        &[("Content-Type", "image/png")],
        b"png-bytes",
    )])
    .await;
    let provider = MockServer::start(vec![reply("200 OK", &[], br#"{"pages":[]}"#)]).await;
    let (source, _calls) = TokenSource::fake("ya29.token", Some("proj-1"));
    let connection = connection(
        Some((&provider.base, OcrInputSource::Deployment)),
        None,
        Vec::new(),
        Secrets::new(),
        settings(None, None),
    );
    let mut request = prepared(connection, Object::new());
    request.document = OcrDocument::DocumentUrl {
        document_url: format!("{}/a.png", file.base),
        extra: BTreeMap::new(),
    };
    ocr(&VertexAiOcrConfig, &client(&source), &request, &NoHooks)
        .await
        .expect("the call succeeds");
    let Json::Object(body) = request_body(&recorded(provider).await[0]) else {
        panic!("the body is an object");
    };
    let Some(Json::Object(document)) = body.get("document") else {
        panic!("the body carries a document");
    };
    assert_eq!(
        document.get("document_url"),
        Some(&Json::Text("data:image/png;base64,cG5nLWJ5dGVz".to_owned()))
    );
    assert_eq!(recorded(file).await.len(), 1);
}

#[tokio::test]
async fn an_authorization_header_replaces_the_token_and_skips_its_source() {
    let (source, calls) = TokenSource::fake("ya29.token", Some("token-project"));
    let headers = vec![
        ("Authorization".to_owned(), "Bearer forwarded".to_owned()),
        ("X-Trace".to_owned(), "trace-1".to_owned()),
    ];
    let request = prepared(
        connection(
            None,
            None,
            headers.clone(),
            Secrets::new(),
            settings(Some("configured-project"), None),
        ),
        Object::new(),
    );
    let environment = common::environment(&request, &client(&source))
        .await
        .expect("the environment resolves");
    assert_eq!(environment.headers, headers);
    assert_eq!(environment.project_id, "configured-project");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_authorization_header_without_a_project_asks_the_token_source_for_it() {
    let (source, calls) = TokenSource::fake("ya29.token", Some("token-project"));
    let request = prepared(
        connection(
            None,
            None,
            vec![("Authorization".to_owned(), "Bearer forwarded".to_owned())],
            Secrets::new(),
            settings(None, None),
        ),
        Object::new(),
    );
    let environment = common::environment(&request, &client(&source))
        .await
        .expect("the environment resolves");
    assert_eq!(environment.project_id, "token-project");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[rstest]
#[case::environment_key(Some("env-key"), None, "env-key")]
#[case::alternate_environment_key(None, Some("alt-key"), "alt-key")]
#[tokio::test]
async fn a_static_key_is_the_bearer_token_without_asking_the_source(
    #[case] vertex_ai_api_key: Option<&str>,
    #[case] vertexai_api_key: Option<&str>,
    #[case] expected: &str,
) {
    let (source, calls) = TokenSource::fake("ya29.token", Some("proj-1"));
    let mut entries = Vec::new();
    if let Some(key) = vertex_ai_api_key {
        entries.push(("VERTEX_AI_API_KEY", key));
    }
    if let Some(key) = vertexai_api_key {
        entries.push(("VERTEXAI_API_KEY", key));
    }
    let request = prepared(
        connection(
            None,
            None,
            Vec::new(),
            secrets(&entries),
            settings(Some("proj-1"), None),
        ),
        Object::new(),
    );
    let environment = common::environment(&request, &client(&source))
        .await
        .expect("the environment resolves");
    assert_eq!(
        environment.headers,
        [("Authorization".to_owned(), format!("Bearer {expected}"))]
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_request_key_is_the_bearer_token() {
    let (source, calls) = TokenSource::fake("ya29.token", Some("proj-1"));
    let request = prepared(
        connection(
            None,
            Some("request-key"),
            Vec::new(),
            Secrets::new(),
            settings(Some("proj-1"), None),
        ),
        Object::new(),
    );
    let environment = common::environment(&request, &client(&source))
        .await
        .expect("the environment resolves");
    assert_eq!(
        environment.headers,
        [("Authorization".to_owned(), "Bearer request-key".to_owned())]
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn the_token_supplies_the_bearer_header_and_the_project() {
    let (source, calls) = TokenSource::fake("ya29.token", Some("token-project"));
    let request = prepared(
        connection(None, None, Vec::new(), Secrets::new(), settings(None, None)),
        Object::new(),
    );
    let environment = common::environment(&request, &client(&source))
        .await
        .expect("the environment resolves");
    assert_eq!(
        environment.headers,
        [("Authorization".to_owned(), "Bearer ya29.token".to_owned())]
    );
    assert_eq!(environment.project_id, "token-project");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_missing_project_is_an_invalid_configuration() {
    let (source, _calls) = TokenSource::fake("ya29.token", None);
    let request = prepared(
        connection(None, None, Vec::new(), Secrets::new(), settings(None, None)),
        Object::new(),
    );
    let error = common::environment(&request, &client(&source))
        .await
        .err()
        .expect("a project is required");
    assert!(
        matches!(
            &error,
            OcrError::Auth(OcrAuthError::InvalidConfiguration { .. })
        ),
        "{error:?}"
    );
}

#[tokio::test]
async fn an_empty_token_is_refused() {
    let (source, _calls) = TokenSource::fake("   ", Some("proj-1"));
    let request = prepared(
        connection(None, None, Vec::new(), Secrets::new(), settings(None, None)),
        Object::new(),
    );
    let error = common::environment(&request, &client(&source))
        .await
        .err()
        .expect("an empty token is an error");
    assert!(
        matches!(
            &error,
            OcrError::Auth(OcrAuthError::InvalidConfiguration { .. })
        ),
        "{error:?}"
    );
}

#[test]
fn the_project_option_names_the_project_and_beats_settings_and_environment() {
    let request = vertex_request(
        params(r#"{"vertex_ai_project":"option-project"}"#),
        settings(Some("settings-project"), None),
        secrets(&[("VERTEXAI_PROJECT", "env-project")]),
    );
    assert_eq!(
        common::vertex_config(&request)
            .expect("the config parses")
            .project
            .as_deref(),
        Some("option-project")
    );
}

#[test]
fn the_environment_project_fills_in_when_nothing_else_names_one() {
    let request = vertex_request(
        Object::new(),
        settings(None, None),
        secrets(&[("VERTEXAI_PROJECT", "env-project")]),
    );
    let config = common::vertex_config(&request).expect("the config parses");
    assert_eq!(
        common::project(&request, &config).as_deref(),
        Some("env-project")
    );
}

#[test]
fn the_settings_project_is_used_before_the_environment() {
    let request = vertex_request(
        Object::new(),
        settings(Some("settings-project"), None),
        secrets(&[("VERTEXAI_PROJECT", "env-project")]),
    );
    assert_eq!(
        common::vertex_config(&request)
            .expect("the config parses")
            .project
            .as_deref(),
        Some("settings-project")
    );
}

#[test]
fn a_blank_project_option_falls_through_to_the_settings() {
    let request = vertex_request(
        params(r#"{"vertex_project":"   "}"#),
        settings(Some("settings-project"), None),
        Secrets::new(),
    );
    assert_eq!(
        common::vertex_config(&request)
            .expect("the config parses")
            .project
            .as_deref(),
        Some("settings-project")
    );
}

#[test]
fn an_empty_settings_project_falls_through_to_the_environment() {
    let request = vertex_request(
        Object::new(),
        settings(Some(""), None),
        secrets(&[("VERTEXAI_PROJECT", "env-project")]),
    );
    let config = common::vertex_config(&request).expect("the config parses");
    assert_eq!(
        common::project(&request, &config).as_deref(),
        Some("env-project")
    );
}

#[test]
fn the_location_option_beats_settings_and_the_environment() {
    let request = vertex_request(
        params(r#"{"vertex_ai_location":"option-location"}"#),
        settings(None, Some("settings-location")),
        secrets(&[
            ("VERTEXAI_LOCATION", "env-location"),
            ("VERTEX_LOCATION", "alternate-location"),
        ]),
    );
    let config = common::vertex_config(&request).expect("the config parses");
    assert_eq!(common::location(&request, &config), "option-location");
}

#[test]
fn the_location_settings_beat_the_environment() {
    let request = vertex_request(
        Object::new(),
        settings(None, Some("settings-location")),
        secrets(&[
            ("VERTEXAI_LOCATION", "env-location"),
            ("VERTEX_LOCATION", "alternate-location"),
        ]),
    );
    let config = common::vertex_config(&request).expect("the config parses");
    assert_eq!(common::location(&request, &config), "settings-location");
}

#[test]
fn the_location_environment_secrets_precede_the_default() {
    let request = vertex_request(
        Object::new(),
        settings(None, None),
        secrets(&[
            ("VERTEXAI_LOCATION", "env-location"),
            ("VERTEX_LOCATION", "alternate-location"),
        ]),
    );
    let config = common::vertex_config(&request).expect("the config parses");
    assert_eq!(common::location(&request, &config), "env-location");
    let alternate = vertex_request(
        Object::new(),
        settings(None, None),
        secrets(&[("VERTEX_LOCATION", "alternate-location")]),
    );
    assert_eq!(common::location(&alternate, &config), "alternate-location");
    let default = vertex_request(Object::new(), settings(None, None), Secrets::new());
    assert_eq!(common::location(&default, &config), "us-central1");
}

#[rstest]
#[case::project("vertex_project", "42")]
#[case::location("vertex_location", "[]")]
fn a_wrong_typed_vertex_option_names_the_field(#[case] name: &str, #[case] value: &str) {
    let request = vertex_request(
        params(&format!(r#"{{"{name}":{value}}}"#)),
        settings(None, None),
        Secrets::new(),
    );
    let error = common::vertex_config(&request)
        .err()
        .expect("the value is not a string");
    assert!(
        matches!(&error, OcrError::RequestField { path } if path == name),
        "{error:?}"
    );
}

#[test]
fn the_location_option_reaches_the_url() {
    let request = vertex_request(
        params(r#"{"vertex_location":"europe-west4"}"#),
        settings(None, None),
        Secrets::new(),
    );
    let environment = VertexOcrEnvironment {
        headers: Vec::new(),
        project_id: "proj-1".to_owned(),
    };
    let url = VertexAiOcrConfig
        .get_complete_url(&request, &Object::new(), &environment)
        .expect("the URL builds");
    assert_eq!(
        url,
        "https://europe-west4-aiplatform.googleapis.com/v1/projects/proj-1/locations/europe-west4/publishers/mistralai/models/mistral-ocr-latest:rawPredict"
    );
}

#[test]
fn the_default_location_reaches_the_url() {
    let request = vertex_request(Object::new(), settings(None, None), Secrets::new());
    let environment = VertexOcrEnvironment {
        headers: Vec::new(),
        project_id: "proj-1".to_owned(),
    };
    let url = VertexAiOcrConfig
        .get_complete_url(&request, &Object::new(), &environment)
        .expect("the URL builds");
    assert_eq!(
        url,
        "https://us-central1-aiplatform.googleapis.com/v1/projects/proj-1/locations/us-central1/publishers/mistralai/models/mistral-ocr-latest:rawPredict"
    );
}

#[test]
fn an_invalid_location_option_fails_before_the_url_is_built() {
    let request = vertex_request(
        params(r#"{"vertex_ai_location":"US"}"#),
        settings(None, None),
        Secrets::new(),
    );
    let environment = VertexOcrEnvironment {
        headers: Vec::new(),
        project_id: "proj-1".to_owned(),
    };
    let error = VertexAiOcrConfig
        .get_complete_url(&request, &Object::new(), &environment)
        .expect_err("the location is not a region");
    assert!(
        matches!(&error, OcrError::RequestField { path } if path == "vertex_location"),
        "{error:?}"
    );
}

#[test]
fn the_body_and_response_equal_the_mistral_provider() {
    let options = params(r#"{"pages":[0],"extension":true}"#);
    let vertex = VertexAiOcrConfig
        .transform_ocr_request(
            document(),
            OcrTransform {
                model: "mistral-ocr-latest",
                optional_params: &options,
                headers: &[],
            },
        )
        .expect("the vertex body builds");
    let mistral = MistralOcrConfig
        .transform_ocr_request(
            document(),
            OcrTransform {
                model: "mistral-ocr-latest",
                optional_params: &options,
                headers: &[],
            },
        )
        .expect("the mistral body builds");
    assert_eq!(
        serde_json::to_string(&vertex).expect("the vertex body is JSON"),
        serde_json::to_string(&mistral).expect("the mistral body is JSON")
    );
    let payload = br#"{"pages":[{"index":0,"markdown":"hello","blocks":[{"type":"title"}]}],"model":"served","document_annotation":{"language":"en"},"usage_info":{"pages_processed":1},"unknown":true}"#;
    let vertex_response = VertexAiOcrConfig
        .transform_ocr_response("requested", payload, RequestFormat::Litellm)
        .expect("the vertex response decodes");
    let mistral_response = MistralOcrConfig
        .transform_ocr_response("requested", payload, RequestFormat::Litellm)
        .expect("the mistral response decodes");
    assert_eq!(vertex_response, mistral_response);
}

#[test]
fn the_bound_request_needs_an_inline_document() {
    let body = |source: &str| -> Json {
        serde_json::from_str(&format!(
            r#"{{"model":"m","document":{{"type":"document_url","document_url":"{source}"}}}}"#
        ))
        .expect("the body is JSON")
    };
    VertexAiOcrConfig
        .validate_request_body(&body("data:application/pdf;base64,YWJj"))
        .expect("the inline document is valid");
    assert!(matches!(
        VertexAiOcrConfig.validate_request_body(&body("https://example.com/a.pdf")),
        Err(OcrError::InvalidDataUri)
    ));
}

#[test]
fn exposes_the_vertex_key_environment_and_secret_names() {
    assert_eq!(
        VertexAiOcrConfig.get_api_key_env_var(),
        Some("VERTEX_AI_API_KEY")
    );
    assert_eq!(
        VertexAiOcrConfig.secret_names(),
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
    assert_eq!(
        VertexAiOcrConfig.get_supported_ocr_params("mistral-ocr-latest"),
        MistralOcrConfig.get_supported_ocr_params("mistral-ocr-latest")
    );
}
