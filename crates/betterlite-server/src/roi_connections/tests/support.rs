#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::header::{AUTHORIZATION, HeaderName};
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode};
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::resource_repository::config::ConfigError;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_roi::estimator::models::{
    ROICompletionMessage, ROICompletionMetadata, ROICompletionRequest, ROICompletionRole,
    ROIResponseFormat, ROIResponseFormatType, ReasoningEffort,
};
use betterlite_roi::settings::ROISettings;
use betterlite_secrets::error::SecretError;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio::task::yield_now;
use tower::ServiceExt as _;
use url::Url;

use crate::auth::auth_identity::AuthContext;
use crate::auth::auth_key_lookup::tests::master_caller;
use crate::roi_connections::gateway_http_client::GatewayHttpClient;
use crate::roi_connections::routes::router;
use crate::roi_connections::state::RoiConnectionsState;
use crate::roi_github::transport::Transport;
use crate::roi_settings::deployments::{RouterCatalog, RouterEstimatorDeployment};
use crate::roi_settings::repository::SettingsRepository;
use crate::roi_settings::store::{SETTINGS_KEY, SettingsCipher};

pub(crate) const SETTINGS: &str = r#"{"github_api_url":"https://api.github.com","github_token":"gh-token","estimator_key":"sk-estimator","repos":["acme/widgets"],"estimator_model":"gpt-4o"}"#;

pub(crate) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(crate) fn stored(estimator_key: &str, repos: &[&str]) -> String {
    let repos_json = serde_json::to_string(repos).expect("repos json");
    format!(
        r#"{{"github_api_url":"https://api.github.com","github_token":"gh-token","estimator_key":"{estimator_key}","repos":{repos_json},"estimator_model":"gpt-4o"}}"#
    )
}

pub(crate) fn base_url() -> Url {
    Url::parse("http://litellm.internal").expect("gateway base url")
}

pub(crate) fn settings() -> ROISettings {
    serde_json::from_str(SETTINGS).expect("test settings")
}

pub(crate) fn caller(subject: &str, roles: &[Role]) -> AuthContext {
    let roles_json = serde_json::to_string(roles).expect("roles json");
    let raw = format!(
        r#"{{"principal_type":"human","subject":"{subject}","roles":{roles_json},"auth_method":"api_key"}}"#
    );
    let principal: Principal = serde_json::from_str(&raw).expect("test principal");
    AuthContext { principal }
}

pub(crate) fn admin() -> AuthContext {
    caller("user", &[Role::PlatformAdmin])
}

pub(crate) fn viewer() -> AuthContext {
    caller("user", &[Role::PlatformViewer])
}

pub(crate) fn member() -> AuthContext {
    caller("user", &[Role::OrgAdmin])
}

pub(crate) fn master() -> AuthContext {
    master_caller()
}

#[derive(Clone)]
pub(crate) struct Answer {
    pub(crate) status: u16,
    pub(crate) body: String,
    pub(crate) headers: Vec<(String, String)>,
}

impl Answer {
    pub(crate) fn ok(body: &str) -> Self {
        Self {
            status: 200,
            body: body.to_owned(),
            headers: Vec::new(),
        }
    }

    pub(crate) fn status(status: u16) -> Self {
        Self {
            status,
            body: String::new(),
            headers: Vec::new(),
        }
    }

    pub(crate) fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Record {
    pub(crate) method: String,
    pub(crate) url: String,
    pub(crate) query: Vec<(String, String)>,
    pub(crate) authorization: Option<String>,
    pub(crate) body: Vec<u8>,
    pub(crate) deadline: Duration,
}

#[derive(Clone)]
pub(crate) struct Wire {
    answers: Arc<Mutex<VecDeque<Answer>>>,
    seen: Arc<Mutex<Vec<Record>>>,
}

impl Wire {
    pub(crate) fn new(answers: Vec<Answer>) -> Self {
        Self {
            answers: Arc::new(Mutex::new(VecDeque::from(answers))),
            seen: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn seen(&self) -> Vec<Record> {
        self.seen.lock().expect("wire lock").clone()
    }
}

impl Transport for Wire {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        yield_now().await;
        self.seen.lock().expect("wire lock").push(record(&request));
        let next = self.answers.lock().expect("wire lock").pop_front();
        let Some(answer) = next else {
            return Err(TransportError::Closed);
        };
        let mut headers = HeaderMap::new();
        for (name, value) in &answer.headers {
            let header_name = HeaderName::from_bytes(name.as_bytes()).expect("header name");
            headers.append(
                header_name,
                HeaderValue::from_str(value).expect("header value"),
            );
        }
        Ok(UpstreamResponse {
            status: answer.status,
            headers,
            body: answer.body.into_bytes(),
        })
    }
}

fn record(request: &OutboundRequest) -> Record {
    let authorization = request
        .headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = match &request.body {
        RequestBody::Json(bytes) => bytes.clone(),
        RequestBody::Empty | RequestBody::Bytes { .. } | RequestBody::Multipart { .. } => {
            Vec::new()
        }
    };
    Record {
        method: request.method.to_string(),
        url: request.url.to_string(),
        query: request.query.clone(),
        authorization,
        body,
        deadline: request.deadline,
    }
}

#[derive(Default)]
pub(crate) struct MemoryRepository {
    params: Mutex<BTreeMap<String, JsonData>>,
}

impl MemoryRepository {
    pub(crate) fn seed(&self, raw: &str) {
        let value: JsonData = serde_json::from_str(raw).expect("seeded parameter");
        self.params
            .lock()
            .expect("repository lock")
            .insert(SETTINGS_KEY.to_owned(), value);
    }
}

#[async_trait]
impl SettingsRepository for MemoryRepository {
    async fn param(&self, name: &str) -> Result<Option<JsonData>, ConfigError> {
        let params = self.params.lock().expect("repository lock");
        Ok(params.get(name).cloned())
    }

    async fn set_param(&self, name: &str, value: JsonData) -> Result<(), ConfigError> {
        self.params
            .lock()
            .expect("repository lock")
            .insert(name.to_owned(), value);
        Ok(())
    }
}

pub(crate) struct PlainCipher;

impl SettingsCipher for PlainCipher {
    fn seal(&self, value: &SecretString) -> Result<String, SecretError> {
        Ok(value.expose_secret().to_owned())
    }

    fn open(&self, value: &str) -> SecretString {
        SecretString::from(value.to_owned())
    }
}

#[derive(Default)]
pub(crate) struct Catalog {
    pub(crate) names: Vec<String>,
}

impl RouterCatalog for Catalog {
    fn model_names(&self) -> Vec<String> {
        self.names.clone()
    }

    fn deployments(&self, _model_group: &str) -> Vec<RouterEstimatorDeployment> {
        Vec::new()
    }
}

pub(crate) fn harness() -> Harness {
    Harness {
        wire: Wire::new(Vec::new()),
        repository: Arc::new(MemoryRepository::default()),
        repository_present: true,
        catalog: Arc::new(Catalog::default()),
        master_key: None,
        stored: SETTINGS.to_owned(),
    }
}

pub(crate) struct Harness {
    pub(crate) wire: Wire,
    pub(crate) repository: Arc<MemoryRepository>,
    pub(crate) repository_present: bool,
    pub(crate) catalog: Arc<Catalog>,
    pub(crate) master_key: Option<SecretString>,
    pub(crate) stored: String,
}

impl Harness {
    pub(crate) fn answers(mut self, answers: Vec<Answer>) -> Self {
        self.wire = Wire::new(answers);
        self
    }

    pub(crate) fn models(mut self, names: &[&str]) -> Self {
        self.catalog = Arc::new(Catalog {
            names: names.iter().copied().map(str::to_owned).collect(),
        });
        self
    }

    pub(crate) fn stored(mut self, raw: &str) -> Self {
        self.stored = raw.to_owned();
        self
    }

    pub(crate) fn master_key(mut self, key: &str) -> Self {
        self.master_key = Some(secret(key));
        self
    }

    pub(crate) fn without_repository(mut self) -> Self {
        self.repository_present = false;
        self
    }

    pub(crate) fn state(&self) -> RoiConnectionsState<Wire> {
        self.repository.seed(&self.stored);
        let concrete_repository = Arc::clone(&self.repository);
        let repository: Arc<dyn SettingsRepository> = concrete_repository;
        let concrete_catalog = Arc::clone(&self.catalog);
        let catalog: Arc<dyn RouterCatalog> = concrete_catalog;
        RoiConnectionsState {
            repository: self.repository_present.then_some(repository),
            catalog: Some(catalog),
            cipher: Arc::new(PlainCipher),
            master_key: self.master_key.clone(),
            http: self.wire.clone(),
            github_transport: None,
            gateway_base: base_url(),
        }
    }

    pub(crate) fn app(&self) -> Router {
        router(self.state())
    }
}

pub(crate) fn gateway(wire: &Wire) -> GatewayHttpClient<Wire> {
    GatewayHttpClient {
        base: base_url(),
        transport: wire.clone(),
    }
}

pub(crate) fn completion_request(reasoning: Option<ReasoningEffort>) -> ROICompletionRequest {
    ROICompletionRequest {
        model: "gpt-4o".to_owned(),
        temperature: 0,
        messages: vec![ROICompletionMessage {
            role: ROICompletionRole::User,
            content: "evidence".to_owned(),
        }],
        response_format: ROIResponseFormat {
            r#type: ROIResponseFormatType::JsonObject,
        },
        max_tokens: 1200,
        metadata: ROICompletionMetadata {
            tags: vec!["litellm-roi-estimator".to_owned()],
            litellm_roi_estimator: true,
        },
        reasoning_effort: reasoning,
    }
}

#[derive(Deserialize)]
pub(crate) struct SentCompletion {
    pub(crate) model: String,
    pub(crate) temperature: u32,
    pub(crate) max_tokens: u32,
    pub(crate) messages: Vec<ROICompletionMessage>,
    pub(crate) response_format: ROIResponseFormat,
    pub(crate) metadata: ROICompletionMetadata,
    pub(crate) reasoning_effort: Option<String>,
}

pub(crate) fn get(path: &str, context: Option<&AuthContext>) -> Request<Body> {
    let request = Request::get(path).body(Body::empty()).expect("get request");
    with_caller(request, context)
}

pub(crate) fn post(path: &str, context: Option<&AuthContext>) -> Request<Body> {
    let request = Request::post(path)
        .body(Body::empty())
        .expect("post request");
    with_caller(request, context)
}

fn with_caller(mut request: Request<Body>, caller: Option<&AuthContext>) -> Request<Body> {
    if let Some(context) = caller {
        request.extensions_mut().insert(context.clone());
    }
    request
}

pub(crate) async fn call(app: Router, request: Request<Body>) -> Reply {
    let response = app.oneshot(request).await.expect("router reply");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("reply body");
    Reply {
        status,
        body: bytes.to_vec(),
    }
}

pub(crate) struct Reply {
    pub(crate) status: StatusCode,
    pub(crate) body: Vec<u8>,
}

impl Reply {
    pub(crate) fn json<T>(&self) -> T
    where
        T: DeserializeOwned,
    {
        serde_json::from_slice(&self.body).expect("json reply")
    }

    pub(crate) fn detail(&self) -> String {
        self.json::<DetailText>().detail
    }

    pub(crate) fn issues(&self) -> Vec<TestIssue> {
        self.json::<IssuesBody>().detail
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8(self.body.clone()).expect("utf-8 reply")
    }
}

#[derive(Deserialize)]
struct DetailText {
    detail: String,
}

#[derive(Deserialize)]
struct IssuesBody {
    detail: Vec<TestIssue>,
}

#[derive(Deserialize)]
pub(crate) struct TestIssue {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) loc: Vec<String>,
    pub(crate) msg: String,
}
