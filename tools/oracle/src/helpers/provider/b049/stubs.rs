use std::sync::Arc;
use std::time::Instant;

use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use secrecy::SecretString;

struct NoTraffic;

impl OidcHttp for NoTraffic {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: String::from("the ragflow helper reads no OIDC document"),
        })
    }
}

impl AzureOidcTokens for NoTraffic {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Err(AzureOidcError::Provider {
            message: String::from("the ragflow helper mints no Azure token"),
        })
    }
}

impl OidcClock for NoTraffic {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

pub(super) fn secret_lookup() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoTraffic),
        azure_tokens: Arc::new(NoTraffic),
        clock: Arc::new(NoTraffic),
        oidc_cache: OidcTokenCache::default(),
    }
}
