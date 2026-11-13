use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use secrecy::SecretString;

use crate::api::responses_provider_contract::records::{Headers, Params, ResponsesUpstream};
use crate::manus_responses::{ManusResponsesConfig, UnknownIdSource};

pub(super) const NOW: i64 = 1_700_000_000;
pub(super) const SUFFIX: &str = "0123abcd";

pub(super) struct FixedIds(pub(super) &'static str);

impl UnknownIdSource for FixedIds {
    fn next_suffix(&self) -> String {
        String::from(self.0)
    }
}

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: String::from("the fixture has no OIDC network"),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Err(AzureOidcError::Provider {
            message: String::from("the fixture has no Azure token provider"),
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

pub(super) fn secrets() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(super) fn globals(api_base: Option<&str>, api_key: Option<&str>) -> GlobalCredentials {
    GlobalCredentials {
        api_base: api_base.map(str::to_owned),
        api_key: api_key.map(SecretString::from),
        xai_key: None,
        inception_key: None,
        lemonade_key: None,
    }
}

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn config(
    globals: GlobalCredentials,
    environment: BTreeMap<String, String>,
    now: i64,
) -> ManusResponsesConfig {
    ManusResponsesConfig {
        globals,
        secrets: secrets(),
        environment,
        clock: Box::new(move || now),
        ids: Box::new(FixedIds(SUFFIX)),
    }
}

pub(super) fn plain_config() -> ManusResponsesConfig {
    config(GlobalCredentials::default(), BTreeMap::new(), NOW)
}

pub(super) fn params(wire: &str) -> Params {
    serde_json::from_str(wire).expect("the fixture is a JSON object")
}

pub(super) fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn response(status: u16, body: &str) -> ResponsesUpstream {
    ResponsesUpstream {
        status_code: status,
        headers: Headers::new(),
        body: body.as_bytes().to_vec(),
    }
}

pub(super) fn headered_response(status: u16, body: &str, headers: Headers) -> ResponsesUpstream {
    ResponsesUpstream {
        status_code: status,
        headers,
        body: body.as_bytes().to_vec(),
    }
}
