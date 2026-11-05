use std::{
    collections::BTreeMap,
    io,
    string::FromUtf8Error,
    sync::{Arc, Mutex},
    time::Duration,
};

use secrecy::SecretString;
use thiserror::Error;

use crate::secret_contracts::SecretFuture;
use cache::{CacheLookup, SecretCache};
use settings::{Options, Settings};

pub mod cache;
pub mod response;
pub mod settings;

const SECRET_MANAGER_ENDPOINT: &str = "https://secretmanager.googleapis.com/v1";

pub type RequestHeaders = BTreeMap<String, SecretString>;

pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

pub trait SecretTransport: Send + Sync {
    fn get<'request>(
        &'request self,
        url: &'request str,
        headers: &'request RequestHeaders,
    ) -> SecretFuture<'request, HttpResponse, io::Error>;
}

pub trait RequestHeaderSource: Send + Sync {
    fn sync_construct_request_headers(&self) -> Result<RequestHeaders, io::Error>;
}

pub trait NativeSecretReader: Send + Sync {
    fn sync_read_secret(&self, secret_name: &str) -> Result<Option<SecretString>, io::Error>;
}

pub struct Dependencies {
    pub transport: Arc<dyn SecretTransport>,
    pub header_source: Arc<dyn RequestHeaderSource>,
    pub native_reader: Option<Arc<dyn NativeSecretReader>>,
    pub clock: Arc<dyn Fn() -> Duration + Send + Sync>,
}

#[derive(Debug, Error)]
pub enum GoogleSecretManagerError {
    #[error(
        "Google Secret Manager requires a project ID, please set 'GOOGLE_SECRET_MANAGER_PROJECT_ID' in your .env"
    )]
    MissingProjectId,
    #[error("Google Secret Manager environment is invalid")]
    Environment(#[source] betterlite_env::EnvError),
    #[error("could not construct Google Secret Manager request headers")]
    Headers(#[source] io::Error),
    #[error("Google Secret Manager request failed")]
    Request(#[source] io::Error),
    #[error("native Google Secret Manager read failed")]
    NativeRead(#[source] io::Error),
    #[error("secret {secret_name} not found in Google Secret Manager, status {status}")]
    Status { secret_name: String, status: u16 },
    #[error("secret {secret_name} not found in Google Secret Manager")]
    PayloadMissing { secret_name: String },
    #[error("Google Secret Manager response is not a valid access response")]
    Response(#[source] serde_json::Error),
    #[error("Google Secret Manager payload is not valid base64")]
    Base64(#[source] base64::DecodeError),
    #[error("Google Secret Manager payload is not valid UTF-8")]
    Utf8(#[source] FromUtf8Error),
    #[error("Google Secret Manager cache is poisoned")]
    CacheState,
}

pub struct GoogleSecretManager {
    pub settings: Settings,
    pub cache: Mutex<SecretCache>,
    pub dependencies: Dependencies,
}

impl GoogleSecretManager {
    pub fn initialize(
        environment: &BTreeMap<String, String>,
        options: Options,
        dependencies: Dependencies,
    ) -> Result<Self, GoogleSecretManagerError> {
        let settings = Settings::resolve(environment, options)?;
        let cache = Mutex::new(SecretCache::new(settings.refresh_interval));
        Ok(Self {
            settings,
            cache,
            dependencies,
        })
    }

    pub async fn get_secret(
        &self,
        secret_name: &str,
    ) -> Result<Option<SecretString>, GoogleSecretManagerError> {
        if let Some(native_reader) = &self.dependencies.native_reader {
            return native_reader
                .sync_read_secret(secret_name)
                .map_err(GoogleSecretManagerError::NativeRead);
        }
        if !self.settings.always_read_secret_manager {
            let now = (self.dependencies.clock)();
            let lookup = self
                .cache
                .lock()
                .map_err(|_poisoned| GoogleSecretManagerError::CacheState)?
                .lookup(secret_name, now);
            if let CacheLookup::Hit(cached) = lookup {
                return Ok(cached);
            }
        }
        self.fetch_and_remember(secret_name).await
    }

    async fn fetch_and_remember(
        &self,
        secret_name: &str,
    ) -> Result<Option<SecretString>, GoogleSecretManagerError> {
        let url = format!(
            "{SECRET_MANAGER_ENDPOINT}/projects/{}/secrets/{secret_name}/versions/latest:access",
            self.settings.project_id
        );
        let headers = self
            .dependencies
            .header_source
            .sync_construct_request_headers()
            .map_err(GoogleSecretManagerError::Headers)?;
        let response = self
            .dependencies
            .transport
            .get(&url, &headers)
            .await
            .map_err(GoogleSecretManagerError::Request)?;
        if response.status != 200 {
            tracing::error!(
                status = response.status,
                body = response.body,
                "Google Secret Manager retrieval error"
            );
            self.remember(secret_name, None)?;
            return Err(GoogleSecretManagerError::Status {
                secret_name: secret_name.to_owned(),
                status: response.status,
            });
        }
        let secret = response::decode_payload(&response.body)?;
        self.remember(secret_name, secret.clone())?;
        if secret.is_none() {
            return Err(GoogleSecretManagerError::PayloadMissing {
                secret_name: secret_name.to_owned(),
            });
        }
        Ok(secret)
    }

    fn remember(
        &self,
        secret_name: &str,
        value: Option<SecretString>,
    ) -> Result<(), GoogleSecretManagerError> {
        let now = (self.dependencies.clock)();
        self.cache
            .lock()
            .map_err(|_poisoned| GoogleSecretManagerError::CacheState)?
            .store(secret_name, value, now);
        Ok(())
    }
}

mod tests;
