#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use betterlite_auth::auth_identity::records::CredentialRef;
use betterlite_auth::auth_identity::records::NetworkContext;
use betterlite_auth::auth_identity::records::Principal;
use betterlite_auth::auth_identity::records::SourceKey;
use betterlite_auth::auth_identity::roles::AuthMethod;
use betterlite_auth::auth_identity::roles::PrincipalType;
use betterlite_auth::auth_identity::roles::Role;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_repository::config::ConfigError;
use betterlite_db::stored_cipher::DbCipher;
use betterlite_roi::settings::DEFAULT_PROMPT;
use betterlite_roi::settings::ROISettings;
use betterlite_secrets::secret_codec::Algorithm;
use betterlite_secrets::secret_codec::CipherSettings;
use betterlite_secrets::secret_codec::OsNonces;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tower::ServiceExt as _;

use crate::auth::auth_identity::AuthContext;
use crate::roi_settings::deployments::RouterCatalog;
use crate::roi_settings::deployments::RouterEstimatorDeployment;
use crate::roi_settings::deployments::RouterEstimatorModelInfo;
use crate::roi_settings::deployments::RouterEstimatorParams;
use crate::roi_settings::patch::encrypt;
use crate::roi_settings::repository::DatabaseSettingsRepository;
use crate::roi_settings::repository::SettingsRepository;
use crate::roi_settings::route::router;
use crate::roi_settings::state::RoiSettingsState;
use crate::roi_settings::store::DEFAULT_GITHUB_API_URL;
use crate::roi_settings::store::SETTINGS_KEY;
use crate::roi_settings::store::StoredSettings;

pub(crate) const SALT: &str = "a sufficiently long roi settings test salt";

pub(crate) fn cipher_settings() -> CipherSettings {
    CipherSettings {
        signing_key: Some(SecretString::from(SALT.to_owned())),
        algorithm: Algorithm::XSalsa20Poly1305,
    }
}

pub(crate) fn cipher() -> DbCipher {
    DbCipher {
        settings: cipher_settings(),
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

pub(crate) fn sealed(value: &str) -> String {
    encrypt(&cipher(), value).expect("seal a test secret")
}

pub(crate) fn deployment(
    model: Option<&str>,
    base_model: Option<&str>,
) -> RouterEstimatorDeployment {
    RouterEstimatorDeployment {
        litellm_params: RouterEstimatorParams {
            model: model.map(str::to_owned),
            base_model: None,
            custom_llm_provider: Some("openai".to_owned()),
        },
        model_info: base_model.map(|base| RouterEstimatorModelInfo {
            base_model: Some(base.to_owned()),
        }),
    }
}

pub(crate) struct FakeCatalog {
    pub(crate) names: Vec<String>,
    pub(crate) deployments: Vec<RouterEstimatorDeployment>,
    pub(crate) requests: Mutex<Vec<String>>,
}

pub(crate) fn fake_catalog() -> FakeCatalog {
    FakeCatalog {
        names: Vec::new(),
        deployments: Vec::new(),
        requests: Mutex::new(Vec::new()),
    }
}

pub(crate) fn roi_settings() -> ROISettings {
    ROISettings {
        github_api_url: DEFAULT_GITHUB_API_URL.to_owned(),
        github_token: SecretString::default(),
        estimator_key: SecretString::default(),
        repos: Vec::new(),
        estimator_model: String::new(),
        estimator_prompt: DEFAULT_PROMPT.to_owned(),
        backfill_days: 7,
        update_interval_minutes: 1440.0_f64,
        identity_map: IndexMap::new(),
    }
}

pub(crate) fn stored_settings() -> StoredSettings {
    StoredSettings {
        github_api_url: DEFAULT_GITHUB_API_URL.to_owned(),
        github_token: String::new(),
        estimator_key: String::new(),
        repos: Vec::new(),
        estimator_model: String::new(),
        estimator_prompt: DEFAULT_PROMPT.to_owned(),
        backfill_days: 7,
        update_interval_minutes: 1440.0,
        identity_map: IndexMap::new(),
    }
}

impl RouterCatalog for FakeCatalog {
    fn model_names(&self) -> Vec<String> {
        self.names.clone()
    }

    fn deployments(&self, model_group: &str) -> Vec<RouterEstimatorDeployment> {
        self.requests
            .lock()
            .expect("catalog lock")
            .push(model_group.to_owned());
        self.deployments.clone()
    }
}

#[derive(Default)]
pub(crate) struct MemoryRepository {
    params: Mutex<BTreeMap<String, JsonData>>,
}

impl MemoryRepository {
    pub(crate) fn seed(&self, name: &str, raw: &str) {
        let value: JsonData = serde_json::from_str(raw).expect("seed json");
        self.params
            .lock()
            .expect("repository lock")
            .insert(name.to_owned(), value);
    }

    pub(crate) fn seed_settings(&self, stored: &StoredSettings) {
        let raw = serde_json::to_string(stored).expect("stored settings json");
        self.seed(SETTINGS_KEY, &raw);
    }

    pub(crate) fn stored(&self) -> Option<StoredSettings> {
        let value = self
            .params
            .lock()
            .expect("repository lock")
            .get(SETTINGS_KEY)
            .cloned()?;
        let raw = serde_json::to_string(&value).expect("stored json");
        Some(serde_json::from_str(&raw).expect("stored settings"))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.params.lock().expect("repository lock").is_empty()
    }
}

#[async_trait]
impl SettingsRepository for MemoryRepository {
    async fn param(&self, name: &str) -> Result<Option<JsonData>, ConfigError> {
        Ok(self
            .params
            .lock()
            .expect("repository lock")
            .get(name)
            .cloned())
    }

    async fn set_param(&self, name: &str, value: JsonData) -> Result<(), ConfigError> {
        self.params
            .lock()
            .expect("repository lock")
            .insert(name.to_owned(), value);
        Ok(())
    }
}

pub(crate) fn catalog(names: Vec<String>) -> Arc<dyn RouterCatalog> {
    let mut catalog = fake_catalog();
    catalog.names = names;
    Arc::new(catalog)
}

pub(crate) fn erased(memory: &Arc<MemoryRepository>) -> Arc<dyn SettingsRepository> {
    Arc::clone(memory)
}

pub(crate) fn disconnected_repository() -> DatabaseSettingsRepository {
    DatabaseSettingsRepository {
        store: Arc::new(PostgresStore { pool: None }),
        availability: Availability::default(),
    }
}

pub(crate) fn settings_state(
    repository: Option<Arc<dyn SettingsRepository>>,
    catalog: Option<Arc<dyn RouterCatalog>>,
) -> RoiSettingsState {
    RoiSettingsState::new(repository, catalog, cipher_settings())
}

pub(crate) fn settings_app(
    repository: Option<Arc<dyn SettingsRepository>>,
    catalog: Option<Arc<dyn RouterCatalog>>,
) -> Router {
    router(settings_state(repository, catalog))
}

pub(crate) fn caller(subject: &str, roles: Vec<Role>) -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::Human,
            subject: subject.to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles,
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key: SourceKey::default(),
        },
    }
}

pub(crate) fn admin_caller() -> AuthContext {
    caller("user", vec![Role::PlatformAdmin])
}

pub(crate) fn get_request(caller: Option<&AuthContext>) -> Request<Body> {
    let request = Request::builder()
        .uri("/roi-calculator/settings")
        .body(Body::empty())
        .expect("get request");
    with_caller(request, caller)
}

pub(crate) fn put_request(caller: Option<&AuthContext>, body: &str) -> Request<Body> {
    let request = Request::builder()
        .method("PUT")
        .uri("/roi-calculator/settings")
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .expect("put request");
    with_caller(request, caller)
}

fn with_caller(mut request: Request<Body>, caller: Option<&AuthContext>) -> Request<Body> {
    if let Some(context) = caller {
        request.extensions_mut().insert(context.clone());
    }
    request
}

pub(crate) async fn call(app: Router, request: Request<Body>) -> Reply {
    let response = app.oneshot(request).await.expect("router response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
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
        serde_json::from_slice(&self.body).expect("json body")
    }

    pub(crate) fn detail(&self) -> String {
        self.json::<DetailText>().detail
    }

    pub(crate) fn issues(&self) -> Vec<TestIssue> {
        self.json::<DetailIssues>().detail
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8(self.body.clone()).expect("utf-8 body")
    }
}

#[derive(Deserialize)]
struct DetailText {
    detail: String,
}

#[derive(Deserialize)]
struct DetailIssues {
    detail: Vec<TestIssue>,
}

#[derive(Deserialize)]
pub(crate) struct TestIssue {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) loc: Vec<TestLoc>,
    pub(crate) msg: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum TestLoc {
    Field(String),
    Index(usize),
}

pub(crate) fn loc_names(issue: &TestIssue) -> Vec<String> {
    let mut names = Vec::new();
    for part in &issue.loc {
        match part {
            TestLoc::Field(field) => names.push(field.clone()),
            TestLoc::Index(index) => names.push(index.to_string()),
        }
    }
    names
}
