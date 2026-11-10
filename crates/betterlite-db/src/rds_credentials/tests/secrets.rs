#![cfg(test)]

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
};
use betterlite_secret_managers::secret_lookup::{SecretLookup, SecretValue};
use secrecy::{ExposeSecret as _, SecretString};

use super::super::secrets::{EnvironmentSecrets, RdsSecrets as _};

struct NoOidcHttp;

impl OidcHttp for NoOidcHttp {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "no HTTP in this test".to_owned(),
        })
    }
}

struct NoAzureTokens;

impl AzureOidcTokens for NoAzureTokens {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

struct FixedClock;

impl OidcClock for FixedClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

#[test]
fn environment_secrets_reads_b8500_values() {
    let lookup = SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidcHttp),
        azure_tokens: Arc::new(NoAzureTokens),
        clock: Arc::new(FixedClock),
        oidc_cache: Mutex::new(HashMap::new()),
    };
    let mut environment = BTreeMap::new();
    environment.insert("REGION".to_owned(), "eu-west-1".to_owned());
    environment.insert("FLAG".to_owned(), "True".to_owned());
    let secrets = EnvironmentSecrets {
        lookup: &lookup,
        environment: &environment,
    };
    let region = secrets.get_secret("REGION").expect("the lookup succeeds");
    assert!(matches!(region, Some(SecretValue::Text(text)) if text.expose_secret() == "eu-west-1"));
    let flag = secrets.get_secret("FLAG").expect("the lookup succeeds");
    assert!(matches!(flag, Some(SecretValue::Bool(true))));
    assert!(matches!(secrets.get_secret("MISSING"), Ok(None)));
}
