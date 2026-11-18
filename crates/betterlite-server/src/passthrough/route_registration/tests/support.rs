use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use base64::engine::general_purpose::STANDARD;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use secrecy::SecretString;

use super::super::*;

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the test has no OIDC transport".to_owned(),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _: &str) -> Result<Option<SecretString>, AzureOidcError> {
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

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) fn raw_headers(json: &str) -> JsonObject {
    serde_json::from_str(json).unwrap()
}

pub(super) fn parse_endpoint(json: &str) -> PassThroughEndpoint {
    serde_json::from_str(json).unwrap()
}

pub(super) fn shared() -> SharedPassThroughRoutes {
    Arc::new(Mutex::new(PassThroughRoutes::default()))
}

pub(super) fn shared_with_route(path: &str, values: &[&str]) -> SharedPassThroughRoutes {
    let mut state = PassThroughRoutes::default();
    state
        .table
        .install(path.to_owned(), method_set(values), false);
    Arc::new(Mutex::new(state))
}

pub(super) fn lock_state(routes: &SharedPassThroughRoutes) -> MutexGuard<'_, PassThroughRoutes> {
    routes.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn method_set(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(super) fn register_with(
    routes: &SharedPassThroughRoutes,
    endpoints: &[PassThroughEndpoint],
    config_file_path: Option<&str>,
    pairs: &[(&str, &str)],
) {
    let secrets = secrets();
    let environment = environment(pairs);
    initialize_pass_through_endpoints(routes, endpoints, config_file_path, &secrets, &environment)
        .unwrap();
}

pub(super) fn registration_error(
    routes: &SharedPassThroughRoutes,
    endpoints: &[PassThroughEndpoint],
    pairs: &[(&str, &str)],
) -> RouteRegistrationError {
    let secrets = secrets();
    let environment = environment(pairs);
    initialize_pass_through_endpoints(routes, endpoints, None, &secrets, &environment).unwrap_err()
}

pub(super) fn registry_keys(routes: &SharedPassThroughRoutes) -> Vec<String> {
    let state = lock_state(routes);
    state
        .registry
        .registrations
        .iter()
        .map(|(key, _)| key.clone())
        .collect()
}

pub(super) fn route_count(routes: &SharedPassThroughRoutes, path: &str) -> usize {
    let state = lock_state(routes);
    state
        .openai_routes
        .iter()
        .filter(|route| *route == path)
        .count()
}

pub(super) fn metadata_auth(routes: &SharedPassThroughRoutes, key: &str) -> Option<bool> {
    let state = lock_state(routes);
    state
        .registry
        .registrations
        .iter()
        .find(|(existing, _)| existing == key)
        .and_then(|(_, metadata)| metadata.auth)
}

pub(super) fn config_path(routes: &SharedPassThroughRoutes, key: &str) -> Option<String> {
    let state = lock_state(routes);
    state.endpoints.get(key)?.config_file_path.clone()
}

pub(super) fn subpath_template(path: &str) -> String {
    format!("{path}/{{subpath:path}}")
}

pub(super) fn basic(public_key: &str, secret_key: &str) -> Json {
    let token = STANDARD.encode(format!("{public_key}:{secret_key}"));
    Json::Text(format!("Basic {token}"))
}
