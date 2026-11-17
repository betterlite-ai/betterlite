use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::pointfive_params::PointFiveInitParams;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use secrecy::{ExposeSecret as _, SecretString};

use super::error::PointFiveError;
use crate::finops::pointfive::types::DEFAULT_API_URL;
use crate::finops::pointfive::upload::PointFiveUploadClient;
use crate::finops::pointfive::upload::sleeper::{TokioSleeper, UploadSleeper};
use crate::finops::pointfive::upload::transport::{DEFAULT_REQUEST_TIMEOUT, UploadHttp};
use crate::finops::pointfive::upload::url_check::UploadUrlCheck;

pub const ENV_REFERENCE_PREFIX: &str = "os.environ/";
pub const POINTFIVE_API_KEY_VARIABLE: &str = "POINTFIVE_API_KEY";
pub const POINTFIVE_API_URL_VARIABLE: &str = "POINTFIVE_API_URL";

pub trait UploadClientSource: Send + Sync {
    fn upload_client(
        &self,
        params: &PointFiveInitParams,
    ) -> Result<PointFiveUploadClient, PointFiveError>;
}

pub struct PointFiveService {
    pub http: Arc<dyn UploadHttp>,
    pub url_check: Arc<dyn UploadUrlCheck>,
    pub sleeper: Arc<dyn UploadSleeper>,
    pub request_timeout: Duration,
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    pub secrets: SecretLookup,
}

impl PointFiveService {
    #[must_use]
    pub fn new(
        http: Arc<dyn UploadHttp>,
        url_check: Arc<dyn UploadUrlCheck>,
        environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    ) -> Self {
        Self {
            http,
            url_check,
            sleeper: Arc::new(TokioSleeper),
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            environment,
            secrets: SecretLookup {
                manager: None,
                oidc_http: Arc::new(NoOidc),
                azure_tokens: Arc::new(NoOidc),
                clock: Arc::new(NoOidc),
                oidc_cache: OidcTokenCache::default(),
            },
        }
    }

    fn resolved_api_key(
        &self,
        params: &PointFiveInitParams,
    ) -> Result<Option<SecretString>, PointFiveError> {
        let configured = self
            .resolved_secret(
                params
                    .api_key
                    .as_ref()
                    .map(secrecy::ExposeSecret::expose_secret),
            )?
            .filter(|key| !key.expose_secret().is_empty());
        if configured.is_some() {
            return Ok(configured);
        }
        self.secret(POINTFIVE_API_KEY_VARIABLE)
    }

    fn resolved_api_url(&self, params: &PointFiveInitParams) -> Result<String, PointFiveError> {
        let configured = self
            .resolved_secret(params.api_url.as_deref())?
            .filter(|url| !url.expose_secret().is_empty());
        if let Some(url) = configured {
            return Ok(url.expose_secret().to_owned());
        }
        if let Some(url) = self.secret(POINTFIVE_API_URL_VARIABLE)? {
            return Ok(url.expose_secret().to_owned());
        }
        Ok(DEFAULT_API_URL.to_owned())
    }

    // _resolved_secret: an os.environ/ reference that resolves to nothing stays unresolved
    fn resolved_secret(&self, value: Option<&str>) -> Result<Option<SecretString>, PointFiveError> {
        let Some(text) = value else {
            return Ok(None);
        };
        if let Some(resolved) = self.secret(text)? {
            return Ok(Some(resolved));
        }
        if text.starts_with(ENV_REFERENCE_PREFIX) {
            return Ok(None);
        }
        Ok(Some(SecretString::from(text)))
    }

    // secret: an empty environment value counts as unset, as Python's truthiness test would
    fn secret(&self, name: &str) -> Result<Option<SecretString>, PointFiveError> {
        Ok(self
            .secrets
            .get_secret_str(&self.environment_variable(name), name, None)?
            .filter(|value| !value.expose_secret().is_empty()))
    }

    fn environment_variable(&self, name: &str) -> BTreeMap<String, String> {
        let variable = name.strip_prefix(ENV_REFERENCE_PREFIX).unwrap_or(name);
        self.environment
            .lookup(variable)
            .map(|value| BTreeMap::from([(variable.to_owned(), value)]))
            .unwrap_or_default()
    }
}

impl UploadClientSource for PointFiveService {
    fn upload_client(
        &self,
        params: &PointFiveInitParams,
    ) -> Result<PointFiveUploadClient, PointFiveError> {
        let api_key = self
            .resolved_api_key(params)?
            .ok_or(PointFiveError::MissingApiKey)?;
        Ok(PointFiveUploadClient {
            api_key,
            api_url: self.resolved_api_url(params)?,
            http: Arc::clone(&self.http),
            url_check: Arc::clone(&self.url_check),
            sleeper: Arc::clone(&self.sleeper),
            max_retries: params.max_upload_retries,
            request_timeout: self.request_timeout,
        })
    }
}

// the pointfive api key is read through get_secret, whose OIDC providers have no transport here
struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "pointfive reads no OIDC secrets".to_owned(),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoOidc {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        i64::try_from(elapsed).unwrap_or(i64::MAX)
    }
}
