use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use betterlite_auth::auth_identity::roles::Role;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_replica::availability::Availability;
use betterlite_db::postgres_store::session::PostgresStore;
use betterlite_db::resource_repository::config::{ConfigError, ConfigRepository};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use betterlite_roi::estimator::models::EstimatorModel;
use betterlite_roi::records::ROIReport;
use betterlite_roi::settings::{
    DEFAULT_PROMPT, ROISettings, ROISettingsResponse, ROISettingsUpdate, SettingsError,
};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    CipherSettings, DecryptPolicy, FailureLog, FailureValue, OsNonces, StoredValue,
};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::de::{DeserializeOwned, IgnoredAny};
use serde::{Deserialize, Serialize};
use serde_path_to_error::Segment;
use thiserror::Error;

use crate::auth::auth_identity::AuthContext;
use crate::tracing_routes::router::unauthenticated;

const SETTINGS_KEY: &str = "roi_calculator_settings";
const REPORT_KEY: &str = "roi_calculator_report";
const DEFAULT_GITHUB_API_URL: &str = "https://api.github.com";
const MASTER_KEY_SUBJECT: &str = "master_key";
const READ_FORBIDDEN: &str = "Only proxy admins can access the ROI Calculator.";
const WRITE_FORBIDDEN: &str = "Only proxy admins can change ROI Calculator settings.";
const NULL_URL: &str = "GitHub API URL cannot be null.";
const INVALID_SETTINGS: &str = "Stored ROI Calculator settings are invalid.";
const INVALID_REPORT: &str = "Stored ROI Calculator report is invalid.";
const INTERNAL: &str = "Internal server error";
const DECRYPT: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Debug,
    value: FailureValue::Nothing,
};

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RouterEstimatorParams {
    pub model: Option<String>,
    pub base_model: Option<String>,
    pub custom_llm_provider: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RouterEstimatorModelInfo {
    pub base_model: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RouterEstimatorDeployment {
    pub litellm_params: RouterEstimatorParams,
    pub model_info: RouterEstimatorModelInfo,
}

#[async_trait]
pub trait SettingsRepository: Send + Sync {
    async fn param(&self, name: &str) -> Result<Option<JsonData>, ConfigError>;
    async fn set_param(&self, name: &str, value: JsonData) -> Result<(), ConfigError>;
}

pub trait RouterCatalog: Send + Sync {
    fn model_names(&self) -> Vec<String>;
    fn deployments(&self, model_group: &str) -> Vec<RouterEstimatorDeployment>;
}

fn config_repository<'db>(
    writer: &'db PostgresStore,
    reader: &'db PostgresStore,
    availability: &'db Availability,
) -> ConfigRepository<'db> {
    let client = PrismaClient {
        writer,
        reader,
        availability,
    };
    ConfigRepository::new(Some(client), true)
}

#[async_trait]
impl SettingsRepository for PostgresStore {
    async fn param(&self, name: &str) -> Result<Option<JsonData>, ConfigError> {
        let availability = Availability::default();
        let repository = config_repository(self, self, &availability);
        let parameter = repository.get_param(name).await?;
        Ok(parameter.map(|param| param.param_value))
    }

    async fn set_param(&self, name: &str, value: JsonData) -> Result<(), ConfigError> {
        let availability = Availability::default();
        let repository = config_repository(self, self, &availability);
        repository.set_param(name, value).await.map(|_| ())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct StoredSettings {
    github_api_url: String,
    github_token: String,
    estimator_key: String,
    repos: Vec<String>,
    estimator_model: String,
    estimator_prompt: String,
    backfill_days: i64,
    update_interval_minutes: f64,
    identity_map: IndexMap<String, String>,
}

impl Default for StoredSettings {
    fn default() -> Self {
        Self {
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
}

#[derive(Default, Deserialize)]
struct PatchPresence {
    #[serde(default, deserialize_with = "deserialize_field_state")]
    github_api_url: FieldState<String>,
    #[serde(default, deserialize_with = "deserialize_field_state")]
    github_token: FieldState<String>,
    #[serde(default, deserialize_with = "deserialize_field_state")]
    estimator_key: FieldState<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
enum FieldState<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

impl<T> FieldState<T> {
    const fn is_present(&self) -> bool {
        !matches!(self, Self::Missing)
    }
}

fn deserialize_field_state<'de, D, T>(deserializer: D) -> Result<FieldState<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.map_or_else(|| FieldState::Null, FieldState::Value))
}

struct PatchedSecrets {
    token: String,
    encrypted_token: String,
    estimator_key: String,
    encrypted_estimator_key: String,
}

#[derive(Clone)]
pub struct RoiSettingsState {
    repository: Option<Arc<dyn SettingsRepository>>,
    catalog: Arc<dyn RouterCatalog>,
    cipher: Arc<DbCipher>,
}

impl RoiSettingsState {
    #[must_use]
    pub fn new(
        repository: Option<Arc<dyn SettingsRepository>>,
        catalog: Arc<dyn RouterCatalog>,
        cipher_settings: CipherSettings,
    ) -> Self {
        let cipher = DbCipher {
            settings: cipher_settings,
            nonces: OsNonces,
            diagnostics: TracingDiagnostics,
        };
        Self {
            repository,
            catalog,
            cipher: Arc::new(cipher),
        }
    }
}

pub fn router(state: RoiSettingsState) -> Router {
    Router::new()
        .route(
            "/roi-calculator/settings",
            get(get_settings).put(update_settings),
        )
        .with_state(state)
}

#[must_use]
pub fn estimator_model(deployment: &RouterEstimatorDeployment) -> Option<EstimatorModel> {
    let parameters = &deployment.litellm_params;
    let model = deployment
        .model_info
        .base_model
        .clone()
        .or_else(|| parameters.base_model.clone())
        .or_else(|| parameters.model.clone())?;
    Some((model, parameters.custom_llm_provider.clone()))
}

pub fn estimator_models(deployments: &[RouterEstimatorDeployment]) -> Vec<EstimatorModel> {
    deployments.iter().filter_map(estimator_model).collect()
}

pub fn router_estimator_models(
    catalog: &dyn RouterCatalog,
    model_group: &str,
) -> Vec<EstimatorModel> {
    let deployments = catalog.deployments(model_group);
    estimator_models(&deployments)
}

fn router_models(catalog: &dyn RouterCatalog) -> Vec<String> {
    let mut names = catalog.model_names();
    names.sort_unstable();
    names.dedup();
    names
}

fn admin(caller: Option<&AuthContext>, write: bool) -> Result<(), RoiSettingsError> {
    let Some(context) = caller else {
        return Err(RoiSettingsError::Unauthenticated);
    };
    let principal = &context.principal;
    let is_master = principal.subject == MASTER_KEY_SUBJECT;
    let is_admin = principal.roles.contains(&Role::PlatformAdmin);
    let is_viewer = !write && principal.roles.contains(&Role::PlatformViewer);
    if !is_master && !is_admin && !is_viewer {
        return Err(RoiSettingsError::Forbidden(if write {
            WRITE_FORBIDDEN
        } else {
            READ_FORBIDDEN
        }));
    }
    Ok(())
}

fn decrypt(cipher: &DbCipher, value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    let stored = StoredValue::<()>::Text(SecretString::from(value.to_owned()));
    match cipher.decrypt_value(stored, SETTINGS_KEY, DECRYPT) {
        Some(StoredValue::Text(plaintext)) => plaintext.expose_secret().to_owned(),
        Some(StoredValue::Other(())) | None => String::new(),
    }
}

fn encrypt(cipher: &DbCipher, value: &str) -> Result<String, RoiSettingsError> {
    if value.is_empty() {
        return Ok(String::new());
    }
    let plaintext = StoredValue::<()>::Text(SecretString::from(value.to_owned()));
    let sealed = cipher
        .encrypt_value(plaintext, None)
        .map_err(|_error| RoiSettingsError::Internal(InternalError::Encryption))?;
    match sealed {
        StoredValue::Text(text) => Ok(text.expose_secret().to_owned()),
        StoredValue::Other(()) => Ok(value.to_owned()),
    }
}
