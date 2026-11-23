use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use super::error::A2aCallError;
use betterlite_a2a::client::{A2A_TIMEOUT_ENV, DEFAULT_A2A_AGENT_TIMEOUT};
use betterlite_a2a::errors::A2AError;
use betterlite_a2a::errors::retries::RetryCounts;
use betterlite_a2a::send::deps::NativeDeps;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_config::types::{Config, ProxyTrust};
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::tls::apply_policy;
use betterlite_models::catalog::BundledCatalog;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::runtime::runtime_counter;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

// The process values every A2A call reads: the environment snapshot, the pool and the TLS policy resolved from it.
pub(crate) struct Setup {
    pub(crate) environment: BTreeMap<String, String>,
    pub(crate) client: ProxiedClient,
    pub(crate) tls: TlsPolicy,
}

// The send paths also count tokens, which needs the shared tokenizer counter.
pub(crate) struct Runtime {
    pub(crate) setup: Setup,
    pub(crate) counter: Counter,
}

impl Setup {
    pub(crate) fn build() -> Result<Self, A2aCallError> {
        let environment = process_environment();
        let config = runtime_config(&environment)?;
        Self::from_config(&config, environment)
    }

    fn from_config(
        config: &Config,
        environment: BTreeMap<String, String>,
    ) -> Result<Self, A2aCallError> {
        let client = proxy_client(&environment)?;
        let tls = TlsPolicy::resolve(config, &environment, None)
            .map_err(|source| A2aCallError::Policy { source })?;
        Ok(Self {
            environment,
            client,
            tls,
        })
    }
}

impl Runtime {
    pub(crate) fn build() -> Result<Self, A2aCallError> {
        let environment = process_environment();
        let config = runtime_config(&environment)?;
        let setup = Setup::from_config(&config, environment)?;
        let catalog = BundledCatalog::default().get();
        let counter = runtime_counter(&config, &Arc::new(setup.client.clone()), &catalog)
            .map_err(|source| A2aCallError::Tokenizer { source })?;
        Ok(Self { setup, counter })
    }

    pub(crate) fn native<'deps>(
        &'deps self,
        trace_ids: &'deps (dyn Fn() -> String + Sync),
    ) -> NativeDeps<'deps> {
        NativeDeps {
            http: &self.setup.client,
            environment: &self.setup.environment,
            tls: &self.setup.tls,
            counter: &self.counter,
            trace_ids,
        }
    }
}

fn proxy_client(environment: &BTreeMap<String, String>) -> Result<ProxiedClient, A2aCallError> {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), environment)
        .map_err(|source| A2aCallError::Proxy { source })?;
    ProxiedClient::outbound(&Arc::new(policy)).map_err(|source| A2aCallError::Client { source })
}

// A Python session has no config file, so the empty document's defaults stand for it.
fn runtime_config(environment: &BTreeMap<String, String>) -> Result<Config, A2aCallError> {
    let loader = ConfigLoader {
        lookup: environment.clone(),
    };
    match loader.load("") {
        Ok(config) => Ok(config),
        Err(source) => Err(A2aCallError::Config { source }),
    }
}

// B4801 hands the configured timeout to a pooled client built with the same TLS policy and proxy route.
pub(crate) fn effective_client(
    pool: &ProxiedClient,
    tls: &TlsPolicy,
    timeout: Duration,
) -> Result<ProxiedClient, A2aCallError> {
    let limits = TransportLimits {
        connect_timeout: timeout,
        read_timeout: timeout,
        ..pool.limits
    };
    let builder =
        apply_policy(base_builder(), tls).map_err(|source| A2aCallError::Tls { source })?;
    let mut client = ProxiedClient::build(builder, &pool.policy, limits)
        .map_err(|source| A2aCallError::Client { source })?;
    client.tracker = pool.tracker.clone();
    client.network_mock = pool.network_mock;
    Ok(client)
}

// The card fetch reads DEFAULT_A2A_AGENT_TIMEOUT when the settings carried no timeout, as create_a2a_client does.
pub(crate) fn resolve_timeout(
    environment: &BTreeMap<String, String>,
    configured: Option<f64>,
) -> Result<Duration, A2aCallError> {
    if let Some(seconds) = configured {
        return Duration::try_from_secs_f64(seconds)
            .map_err(|_source| A2aCallError::Timeout { field: "timeout" });
    }
    let Some(raw) = environment
        .get(A2A_TIMEOUT_ENV)
        .filter(|value| !value.is_empty())
    else {
        return Ok(DEFAULT_A2A_AGENT_TIMEOUT);
    };
    let parsed = raw
        .parse::<f64>()
        .ok()
        .and_then(|value| Duration::try_from_secs_f64(value).ok());
    parsed.ok_or_else(|| A2aCallError::A2A(invalid_timeout(raw)))
}

// B4801 states a bad DEFAULT_A2A_AGENT_TIMEOUT as an A2AError, so both client and card paths say the same.
fn invalid_timeout(raw: &str) -> A2AError {
    A2AError::Base {
        message: format!("{A2A_TIMEOUT_ENV} {raw:?} is not a valid number of seconds"),
        model: None,
        debug_info: None,
        retries: RetryCounts::default(),
    }
}

pub(crate) fn card_headers(
    extra: Option<&BTreeMap<String, SecretString>>,
) -> Result<HeaderMap, A2aCallError> {
    extra
        .into_iter()
        .flatten()
        .map(|(name, value)| {
            let header_name =
                HeaderName::try_from(name.as_str()).map_err(|source| A2aCallError::HeaderName {
                    name: name.clone(),
                    source,
                })?;
            let header_value =
                HeaderValue::try_from(value.expose_secret().as_str()).map_err(|source| {
                    A2aCallError::HeaderValue {
                        name: name.clone(),
                        source,
                    }
                })?;
            Ok((header_name, header_value))
        })
        .collect()
}

pub(crate) fn trace_ids(trace_id: Option<String>) -> impl Fn() -> String + Sync {
    move || {
        trace_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
    }
}

mod tests;
