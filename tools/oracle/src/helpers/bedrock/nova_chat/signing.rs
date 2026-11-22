use std::sync::Arc;
use std::time::Instant;

use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use secrecy::SecretString;

struct NoOidcHttp;

impl OidcHttp for NoOidcHttp {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: String::from("the oracle helper has no OIDC network"),
        })
    }
}

struct NoAzureTokens;

impl AzureOidcTokens for NoAzureTokens {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Err(AzureOidcError::Provider {
            message: String::from("the oracle helper has no Azure token provider"),
        })
    }
}

struct FixedClock(Instant);

impl OidcClock for FixedClock {
    fn now(&self) -> Instant {
        self.0
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

// Only the process environment backs the lookup, so a reference the call does not reach can never fail.
pub(super) fn secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidcHttp),
        azure_tokens: Arc::new(NoAzureTokens),
        clock: Arc::new(FixedClock(Instant::now())),
        oidc_cache: OidcTokenCache::default(),
    }
}
