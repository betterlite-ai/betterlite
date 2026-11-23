use std::collections::BTreeMap;
use std::sync::Arc;

use super::error::A2aCallError;
use betterlite_a2a::send::deps::NativeDeps;
use betterlite_config::loader::ConfigLoader;
use betterlite_config::loader::startup::process_environment;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::TlsPolicy;
use betterlite_config::types::{Config, ProxyTrust};
use betterlite_http::proxy::ProxiedClient;
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
            let header_value = HeaderValue::try_from(value.expose_secret()).map_err(|source| {
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
