use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use secrecy::SecretString;
use tokio::runtime::{Builder, Runtime};

use crate::helpers::{HelperError, OrHelperError as _};

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the fixture has no OIDC transport".to_owned(),
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
        0
    }
}

pub(super) fn secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(super) fn runtime() -> Result<Runtime, Box<HelperError>> {
    let mut builder = Builder::new_current_thread();
    builder.enable_all();
    builder.build().or_value_error()
}

pub(super) fn epoch(seconds: i64) -> Result<SystemTime, Box<HelperError>> {
    let bounded = u64::try_from(seconds).or_value_error()?;
    UNIX_EPOCH
        .checked_add(Duration::from_secs(bounded))
        .ok_or("now is out of range")
        .or_value_error()
}
