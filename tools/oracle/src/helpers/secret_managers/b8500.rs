use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::SecretValue;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{Helper, HelperResult};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    function: Function,
    secret_name: String,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    default_value: Option<DefaultValue>,
}

#[derive(Clone, Copy, Deserialize)]
enum Function {
    #[serde(rename = "get_secret")]
    GetSecret,
    #[serde(rename = "get_secret_str")]
    Str,
    #[serde(rename = "get_secret_bool")]
    Bool,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum DefaultValue {
    Text(String),
    Bool(bool),
}

impl DefaultValue {
    fn to_secret_value(&self) -> SecretValue {
        match self {
            Self::Text(text) => SecretValue::Text(SecretString::from(text.clone())),
            Self::Bool(value) => SecretValue::Bool(*value),
        }
    }

    const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            Self::Text(_) => None,
        }
    }
}

struct NoOidcHttp;

impl OidcHttp for NoOidcHttp {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the oracle helper has no OIDC network".to_owned(),
        })
    }
}

struct NoAzureTokens;

impl AzureOidcTokens for NoAzureTokens {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Err(AzureOidcError::Provider {
            message: "the oracle helper has no Azure token provider".to_owned(),
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

fn output(result: Result<Option<SecretValue>, SecretLookupError>) -> Value {
    let (value, error) = match result {
        Ok(Some(SecretValue::Text(text))) => (Value::from(text.expose_secret()), None),
        Ok(Some(SecretValue::Bool(value))) => (Value::from(value), None),
        Ok(None) => (Value::Null, None),
        Err(error) => (Value::Null, Some(error.to_string())),
    };
    serde_json::json!({"value": value, "error": error})
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let lookup = SecretLookup {
            manager: None,
            oidc_http: Arc::new(NoOidcHttp),
            azure_tokens: Arc::new(NoAzureTokens),
            clock: Arc::new(FixedClock(Instant::now())),
            oidc_cache: OidcTokenCache::default(),
        };
        let fallback = input.default_value.as_ref();
        let result = match input.function {
            Function::GetSecret => lookup.get_secret(
                &input.environment,
                &input.secret_name,
                fallback.map(DefaultValue::to_secret_value),
            ),
            Function::Str => lookup
                .get_secret_str(
                    &input.environment,
                    &input.secret_name,
                    fallback.map(DefaultValue::to_secret_value),
                )
                .map(|value| value.map(SecretValue::Text)),
            Function::Bool => lookup
                .get_secret_bool(
                    &input.environment,
                    &input.secret_name,
                    fallback.and_then(DefaultValue::as_bool),
                )
                .map(|value| value.map(SecretValue::Bool)),
        };
        Ok(output(result))
    }
}
