pub(crate) mod tests;

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::langfuse_settings::Settings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use secrecy::{ExposeSecret, SecretString};

use crate::dynamic_logger_cache::{DynamicLoggerCache, DynamicLoggerCacheError};
use crate::langfuse_logger::langfuse::{LangFuseLogger, LangfuseLoggingConfig};
use crate::langfuse_logger::{LangfuseLoggerError, LangfuseLoggerRequest, LangfuseServices};
use crate::langfuse_settings::{CredentialRequest, LangfuseSettingsError, resolve_environment};

const SERVICE_NAME: &str = "langfuse";
const DEFAULT_FLUSH_INTERVAL: u64 = 1;
const DYNAMIC_CREDENTIAL_FIELDS: [&str; 4] = [
    "langfuse_host",
    "langfuse_public_key",
    "langfuse_secret",
    "langfuse_secret_key",
];

#[derive(Debug, thiserror::Error)]
pub enum LangfuseSelectionError {
    #[error("standard_callback_dynamic_params must be an object")]
    ParamsType,
    #[error("standard_callback_dynamic_params.{field} must be a string")]
    FieldType { field: &'static str },
    #[error(transparent)]
    Settings(#[from] LangfuseSettingsError),
    #[error(transparent)]
    Logger(#[from] LangfuseLoggerError),
    #[error(transparent)]
    Cache(#[from] DynamicLoggerCacheError),
}

// litellm.integrations.langfuse.langfuse_handler.LangFuseHandler: no state, only entry points
pub struct LangFuseHandler;

impl LangFuseHandler {
    // LangFuseHandler.get_langfuse_logger_for_request
    pub async fn logger_for_request(
        services: &LangfuseServices,
        cache: &DynamicLoggerCache<Arc<LangFuseLogger>>,
        dynamic_params: &Json,
        global: Option<Arc<LangFuseLogger>>,
    ) -> Result<Arc<LangFuseLogger>, LangfuseSelectionError> {
        if !Self::has_dynamic_credentials(&services.settings, dynamic_params)? {
            return Self::global_logger(services, cache, global).await;
        }
        let config = Self::dynamic_logging_config(&services.settings, dynamic_params)?;
        let credentials = credential_map(&config);
        if let Some(cached) = cache.get_cache(&credentials, SERVICE_NAME)? {
            return Ok(cached);
        }
        Self::logger_from_credentials(services, cache, credentials, config).await
    }

    // LangFuseHandler._return_global_langfuse_logger
    async fn global_logger(
        services: &LangfuseServices,
        cache: &DynamicLoggerCache<Arc<LangFuseLogger>>,
        global: Option<Arc<LangFuseLogger>>,
    ) -> Result<Arc<LangFuseLogger>, LangfuseSelectionError> {
        if let Some(global) = global {
            return Ok(global);
        }
        let empty = BTreeMap::new();
        if let Some(cached) = cache.get_cache(&empty, SERVICE_NAME)? {
            return Ok(cached);
        }
        Self::logger_from_credentials(services, cache, empty, LangfuseLoggingConfig::default())
            .await
    }

    // LangFuseHandler._create_langfuse_logger_from_credentials: build the logger, then cache it
    async fn logger_from_credentials(
        services: &LangfuseServices,
        cache: &DynamicLoggerCache<Arc<LangFuseLogger>>,
        credentials: BTreeMap<String, SecretString>,
        config: LangfuseLoggingConfig,
    ) -> Result<Arc<LangFuseLogger>, LangfuseSelectionError> {
        let secret = config.langfuse_secret.as_ref();
        let request = LangfuseLoggerRequest {
            credentials: CredentialRequest {
                public_key: config.langfuse_public_key.as_deref(),
                secret: secret.map(ExposeSecret::expose_secret),
                secret_key: None,
                host: config.langfuse_host.as_deref(),
                allow_env_credentials: config.langfuse_host.is_none(),
            },
            environment: config.langfuse_environment.as_deref(),
            flush_interval: DEFAULT_FLUSH_INTERVAL,
        };
        let logger = Arc::new(LangFuseLogger::new(services, &request).await?);
        cache.set_cache(&credentials, SERVICE_NAME, Arc::clone(&logger))?;
        Ok(logger)
    }

    // LangFuseHandler.get_dynamic_langfuse_logging_config
    pub fn dynamic_logging_config(
        settings: &Settings,
        dynamic_params: &Json,
    ) -> Result<LangfuseLoggingConfig, LangfuseSelectionError> {
        let fields = dynamic_fields(dynamic_params)?;
        let secret = credential_field(fields, "langfuse_secret")?;
        let secret_key = credential_field(fields, "langfuse_secret_key")?;
        Ok(LangfuseLoggingConfig {
            langfuse_secret: secret
                .filter(|value| !value.is_empty())
                .or(secret_key)
                .map(SecretString::from),
            langfuse_public_key: credential_field(fields, "langfuse_public_key")?,
            langfuse_host: credential_field(fields, "langfuse_host")?,
            langfuse_environment: Self::meaningful_dynamic_environment(settings, dynamic_params)?,
        })
    }

    // LangFuseHandler._meaningful_dynamic_environment: only an environment that changes behavior survives
    pub fn meaningful_dynamic_environment(
        settings: &Settings,
        dynamic_params: &Json,
    ) -> Result<Option<String>, LangfuseSelectionError> {
        let fields = dynamic_fields(dynamic_params)?;
        let Some(raw) = credential_field(fields, "langfuse_environment")? else {
            return Ok(None);
        };
        let value = raw.trim();
        if value.is_empty() {
            return Ok(None);
        }
        if settings.tracing_environment.as_deref() == Some(value) {
            return Ok(None);
        }
        let deployment = resolve_environment(settings, None)?;
        if deployment.as_deref() == Some(value) {
            return Ok(None);
        }
        Ok(Some(value.to_owned()))
    }

    // LangFuseHandler._dynamic_langfuse_credentials_are_passed: presence, not content, decides
    pub fn has_dynamic_credentials(
        settings: &Settings,
        dynamic_params: &Json,
    ) -> Result<bool, LangfuseSelectionError> {
        let fields = dynamic_fields(dynamic_params)?;
        if DYNAMIC_CREDENTIAL_FIELDS
            .iter()
            .any(|name| is_set(fields, name))
        {
            return Ok(true);
        }
        Ok(Self::meaningful_dynamic_environment(settings, dynamic_params)?.is_some())
    }
}

fn is_set(fields: &IndexMap<String, Json>, name: &str) -> bool {
    fields
        .get(name)
        .is_some_and(|value| !matches!(value, Json::Null))
}

fn dynamic_fields(
    dynamic_params: &Json,
) -> Result<&IndexMap<String, Json>, LangfuseSelectionError> {
    match dynamic_params {
        Json::Object(fields) => Ok(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(LangfuseSelectionError::ParamsType)
        }
    }
}

fn credential_field(
    fields: &IndexMap<String, Json>,
    name: &'static str,
) -> Result<Option<String>, LangfuseSelectionError> {
    match fields.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(LangfuseSelectionError::FieldType { field: name })
        }
    }
}

// LangfuseLoggingConfig as the dict the in-memory cache keys on
fn credential_map(config: &LangfuseLoggingConfig) -> BTreeMap<String, SecretString> {
    let mut credentials = BTreeMap::new();
    insert_credential(
        &mut credentials,
        "langfuse_secret",
        config.langfuse_secret.clone(),
    );
    insert_credential(
        &mut credentials,
        "langfuse_public_key",
        config.langfuse_public_key.clone().map(SecretString::from),
    );
    insert_credential(
        &mut credentials,
        "langfuse_host",
        config.langfuse_host.clone().map(SecretString::from),
    );
    insert_credential(
        &mut credentials,
        "langfuse_environment",
        config.langfuse_environment.clone().map(SecretString::from),
    );
    credentials
}

fn insert_credential(
    credentials: &mut BTreeMap<String, SecretString>,
    name: &str,
    value: Option<SecretString>,
) {
    if let Some(value) = value {
        credentials.insert(name.to_owned(), value);
    }
}
