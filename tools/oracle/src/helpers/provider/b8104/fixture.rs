use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_provider::manus_responses::{ManusResponsesConfig, UuidIdSource};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use secrecy::SecretString;

use super::Sdk;

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: String::from("the oracle has no OIDC network"),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Err(AzureOidcError::Provider {
            message: String::from("the oracle has no Azure token provider"),
        })
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

fn secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(super) fn config(
    environment: &BTreeMap<String, String>,
    sdk: &Sdk,
    now: i64,
) -> ManusResponsesConfig {
    ManusResponsesConfig {
        globals: GlobalCredentials {
            api_base: sdk.api_base.clone(),
            api_key: sdk.api_key.as_deref().map(SecretString::from),
            xai_key: None,
            inception_key: None,
            lemonade_key: None,
        },
        secrets: secrets(),
        environment: environment.clone(),
        clock: Box::new(move || now),
        ids: Box::new(UuidIdSource),
    }
}

pub(super) const fn empty_sdk() -> Sdk {
    Sdk {
        api_base: None,
        api_key: None,
    }
}
