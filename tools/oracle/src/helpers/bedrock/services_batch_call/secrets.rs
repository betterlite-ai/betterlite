use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use secrecy::SecretString;
use serde_json::value::{RawValue, to_raw_value};

use super::CredentialInput;

struct NoPorts;

impl OidcHttp for NoPorts {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the fixture has no OIDC transport".to_owned(),
        })
    }
}

impl AzureOidcTokens for NoPorts {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoPorts {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

pub(super) fn fixture_secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoPorts),
        azure_tokens: Arc::new(NoPorts),
        clock: Arc::new(NoPorts),
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(super) fn credentials_of(input: &CredentialInput) -> Credentials {
    Credentials {
        access_key_id: SecretString::from(input.access_key_id.clone()),
        secret_access_key: SecretString::from(input.secret_access_key.clone()),
        session_token: input
            .session_token
            .as_ref()
            .map(|token| SecretString::from(token.clone())),
    }
}

pub(super) fn async_parameters(
    input: &CredentialInput,
) -> Result<BTreeMap<String, Box<RawValue>>, Box<HelperError>> {
    let mut parameters = BTreeMap::new();
    parameters.insert(
        "aws_access_key_id".to_owned(),
        to_raw_value(&input.access_key_id).or_value_error()?,
    );
    parameters.insert(
        "aws_secret_access_key".to_owned(),
        to_raw_value(&input.secret_access_key).or_value_error()?,
    );
    if let Some(token) = &input.session_token {
        parameters.insert(
            "aws_session_token".to_owned(),
            to_raw_value(token).or_value_error()?,
        );
    }
    Ok(parameters)
}
