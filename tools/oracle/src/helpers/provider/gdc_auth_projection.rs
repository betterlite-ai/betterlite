use std::collections::BTreeMap;
use std::io::{Error as IoError, ErrorKind};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::Section;
use betterlite_config::sections::gdc_settings::{Raw, Settings};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::gdc_policy::{caching_enabled, read_env_bool};
use betterlite_prompts::arguments::json::Object;
use betterlite_provider::gdc_auth::authenticator::{GdcAuth, GdcAuthParts, GdcValidation};
use betterlite_provider::gdc_auth::cache::TokenCache;
use betterlite_provider::gdc_auth::credential::FixtureCredentials;
use betterlite_provider::gdc_auth::headers::Headers;
use betterlite_provider::gdc_auth::key::{GdcKey, decode_gdc_key};
#[rustfmt::skip]
use betterlite_provider::gdc_auth::transport::{CaCertificates, CaError, GdcClock, GdcJitter, GdcSleeper, TokenExchangeRequest, TokenExchangeResponse, TokenTransport, TransportFailure};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

const MODEL: &str = "gdc/gemini-2.5-pro";
const FIXTURE_TOKEN: &str = "fixture-token";

mod tests;

pub(crate) struct Op;

#[rustfmt::skip]
#[derive(Debug, Deserialize)] #[serde(deny_unknown_fields)]
pub(crate) struct Input { scenarios: Vec<Scenario> }

#[rustfmt::skip]
#[derive(Debug, Deserialize)] #[serde(deny_unknown_fields)]
struct Scenario {
    config: Value,
    #[serde(default)] environment: BTreeMap<String, String>,
    #[serde(default)] keys: Vec<KeyCall>,
    #[serde(default)] validations: Vec<ValidationCall>,
}

#[rustfmt::skip]
#[derive(Debug, Deserialize)] #[serde(deny_unknown_fields)]
struct KeyCall { api_key: String }

#[rustfmt::skip]
#[derive(Debug, Deserialize)] #[serde(deny_unknown_fields)]
struct HeaderPair { name: String, value: String }

#[rustfmt::skip]
#[derive(Debug, Deserialize)] #[serde(deny_unknown_fields)]
struct ValidationCall {
    api_key: String,
    #[serde(default)] base: Option<String>,
    deployment: usize,
    #[serde(default)] headers: Vec<HeaderPair>,
    #[serde(default)] optional_params: Object,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let scenarios = input
            .scenarios
            .iter()
            .map(scenario)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({ "scenarios": scenarios }))
    }
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

#[rustfmt::skip]
fn scenario(input: &Scenario) -> Result<Value, Box<HelperError>> {
    let raw = serde_json::from_value::<Raw>(input.config.clone()).map_err(|error| failure(error.to_string()))?;
    let loaded = Section::convert(raw, &input.environment).map_err(|error| failure(error.to_string()))?;
    let auth = fixture_auth();
    let keys = input.keys.iter().map(key_outcome).collect::<Vec<_>>();
    let validations = input.validations.iter().map(|call| validation_outcome(call, &loaded, &auth)).collect::<Result<Vec<_>, _>>()?;
    Ok(json!({ "keys": keys, "validations": validations }))
}

fn key_outcome(call: &KeyCall) -> Value {
    match decode_gdc_key(call.api_key.as_bytes(), MODEL) {
        Ok(GdcKey::Bearer(_token)) => json!({ "ok": true, "kind": "bearer" }),
        Ok(GdcKey::ServiceAccount(_credential)) => json!({ "ok": true, "kind": "service_account" }),
        Err(error) => json!({ "ok": false, "status": error.class().metadata().status_code }),
    }
}

#[rustfmt::skip]
fn validation_outcome(call: &ValidationCall, settings: &Settings, auth: &GdcAuth) -> Result<Value, Box<HelperError>> {
    let deployment = settings.deployments.get(call.deployment).ok_or_else(|| failure(format!("deployment {} is not configured", call.deployment)))?;
    let ssl_verify = read_env_bool("SSL_VERIFY", deployment.ssl_verify.as_ref(), settings.ssl_verify_environment.as_deref(), true);
    let caching = caching_enabled(&read_env_bool("GDC_TOKEN_CACHING", deployment.gdc_token_caching.as_ref(), settings.gdc_token_caching_environment.as_deref(), false));
    let base = [call.base.as_deref(), settings.global.gdc_api_base.as_deref(), settings.global.api_base.as_deref()].into_iter().flatten().find(|base| !base.is_empty());
    let api_key = SecretString::from(call.api_key.clone());
    let headers = call.headers.iter().map(|pair| (pair.name.clone(), SecretString::from(pair.value.clone()))).collect::<Headers>();
    let validation = GdcValidation { model: MODEL, api_base: base, api_key: Some(&api_key), headers, optional_params: &call.optional_params, call: deployment, global: &settings.global, ssl_verify, caching, deadline: Duration::from_secs(30), source_identity: "source-handle" };
    let runtime = tokio::runtime::Builder::new_current_thread().build().map_err(|error| failure(error.to_string()))?;
    match runtime.block_on(auth.validate_environment(validation)) {
        Ok(projected) => Ok(json!({ "ok": true, "headers": exposed(&projected), "cache": auth.cache.cache_keys() })),
        Err(error) => Ok(json!({ "ok": false, "status": error.class().metadata().status_code })),
    }
}

fn exposed(headers: &Headers) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), value.expose_secret().to_owned()))
        .collect()
}

fn fixture_auth() -> GdcAuth {
    GdcAuth {
        parts: GdcAuthParts {
            credentials: Arc::new(FixtureCredentials {
                token: SecretString::from(FIXTURE_TOKEN.to_owned()),
            }),
            transport: Arc::new(UnusedTransport),
            ca: Arc::new(UnusedCa),
            clock: Arc::new(FixedClock),
            sleeper: Arc::new(UnusedSleeper),
            jitter: Arc::new(UnusedJitter),
        },
        cache: TokenCache::default(),
    }
}

struct FixedClock;

impl GdcClock for FixedClock {
    fn unix_seconds(&self) -> i64 {
        1_700_000_000
    }

    fn monotonic_millis(&self) -> u64 {
        0
    }
}

struct UnusedTransport;

#[async_trait]
impl TokenTransport for UnusedTransport {
    async fn exchange(
        &self,
        _request: TokenExchangeRequest,
    ) -> Result<TokenExchangeResponse, TransportFailure> {
        Err(TransportFailure::Request {
            source: Box::new(TransportError::Closed),
        })
    }
}

struct UnusedCa;

#[async_trait]
impl CaCertificates for UnusedCa {
    async fn load(&self, path: &str) -> Result<Vec<u8>, CaError> {
        Err(CaError::Read {
            path: path.to_owned(),
            source: IoError::from(ErrorKind::NotFound),
        })
    }
}

struct UnusedSleeper;

#[async_trait]
impl GdcSleeper for UnusedSleeper {
    async fn sleep(&self, _delay: Duration) {}
}

struct UnusedJitter;

impl GdcJitter for UnusedJitter {
    fn uniform_millis(&self, low_inclusive: u64, _high_inclusive: u64) -> u64 {
        low_inclusive
    }
}
