use std::collections::BTreeMap;
use std::str::FromStr as _;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::identity::provider_id::ProviderId;
use reqwest::blocking::{Client as BlockingClient, ClientBuilder as BlockingClientBuilder};
use reqwest::redirect::Policy;
use reqwest::retry::never;
use reqwest::{Client, ClientBuilder};

use super::{ClientError, DEFAULT_PROVIDER, Params};
use crate::proxy::base_builder;
use crate::proxy::routing::with_forward_routes;
use crate::tls::apply_policy;

pub(crate) mod asynchronous;

// LiteLLM's _DEFAULT_TIMEOUT: 600 seconds for the whole request and 5 to connect
const CLIENT_TIMEOUT: Duration = Duration::from_secs(600);
const CLIENT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ClientKey {
    pub(crate) provider: ProviderId,
    pub(crate) ssl_verify: Option<bool>,
}

// _get_httpx_client and get_async_httpx_client: one client per TLS setting, and per provider for the async form
pub struct HttpClients {
    tls: TlsPolicy,
    pub(crate) proxy: Arc<ProxyPolicy>,
    pub(crate) async_clients: BTreeMap<ClientKey, Client>,
    pub(crate) sync_clients: BTreeMap<Option<bool>, BlockingClient>,
}

impl HttpClients {
    #[must_use]
    pub fn new(tls: TlsPolicy, proxy: Arc<ProxyPolicy>) -> Self {
        Self {
            tls,
            proxy,
            async_clients: BTreeMap::new(),
            sync_clients: BTreeMap::new(),
        }
    }

    // _sync_client: the supplied client, or one built with the ssl_verify litellm_params carries
    pub fn sync_client(
        &mut self,
        litellm_params: &Params,
        supplied: Option<BlockingClient>,
    ) -> Result<BlockingClient, ClientError> {
        if let Some(client) = supplied {
            return Ok(client);
        }
        let tls = &self.tls;
        let proxy = &self.proxy;
        cached(&mut self.sync_clients, ssl_verify(litellm_params)?, |key| {
            BlockingClientBuilder::from(client_builder(tls, proxy, key)?)
                .build()
                .map_err(ClientError::Build)
        })
    }
}

// _async_client: the supplied client, or one built for custom_llm_provider, gemini when it names none
pub fn async_client(
    clients: &mut HttpClients,
    litellm_params: &Params,
    supplied: Option<Client>,
) -> Result<Client, ClientError> {
    if let Some(client) = supplied {
        return Ok(client);
    }
    let provider = provider(litellm_params)?;
    asynchronous::async_client_for(clients, provider, litellm_params, None)
}

fn cached<ClientType, Key, Build>(
    cache: &mut BTreeMap<Key, ClientType>,
    key: Key,
    build: Build,
) -> Result<ClientType, ClientError>
where
    ClientType: Clone,
    Key: Copy + Ord,
    Build: FnOnce(Key) -> Result<ClientType, ClientError>,
{
    if let Some(client) = cache.get(&key) {
        return Ok(client.clone());
    }
    let client = build(key)?;
    cache.insert(key, client.clone());
    Ok(client)
}

fn ssl_verify(litellm_params: &Params) -> Result<Option<bool>, ClientError> {
    match litellm_params.get("ssl_verify") {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(ClientError::FieldType {
            field: "ssl_verify",
            expected: "a boolean",
        }),
    }
}

fn provider(litellm_params: &Params) -> Result<ProviderId, ClientError> {
    let name = match litellm_params.get("custom_llm_provider") {
        None | Some(JsonValue::Null) => return Ok(DEFAULT_PROVIDER),
        Some(JsonValue::Text(name)) => name,
        Some(_) => {
            return Err(ClientError::FieldType {
                field: "custom_llm_provider",
                expected: "a string",
            });
        }
    };
    if name.is_empty() {
        return Ok(DEFAULT_PROVIDER);
    }
    ProviderId::from_str(name).map_err(|_| ClientError::UnknownProvider {
        provider: name.clone(),
    })
}

fn client_builder(
    tls: &TlsPolicy,
    proxy: &Arc<ProxyPolicy>,
    ssl_verify: Option<bool>,
) -> Result<ClientBuilder, ClientError> {
    let policy = client_policy(tls, ssl_verify);
    let builder = apply_policy(base_builder(), &policy)?
        .no_proxy()
        .redirect(Policy::none())
        .retry(never())
        .connect_timeout(CLIENT_CONNECT_TIMEOUT)
        .timeout(CLIENT_TIMEOUT);
    Ok(with_forward_routes(builder, proxy))
}

// the ssl_verify litellm_params carries overrides the resolved policy the way a deployment setting does
pub(crate) fn client_policy(tls: &TlsPolicy, ssl_verify: Option<bool>) -> TlsPolicy {
    let verification = match ssl_verify {
        Some(false) => Verification::Disabled,
        Some(true) => tls.ca_file.as_ref().map_or(Verification::Enabled, |path| {
            Verification::CaFile(path.clone())
        }),
        None => tls.verification.clone(),
    };
    TlsPolicy {
        verification,
        ca_file: tls.ca_file.clone(),
        certificate: tls.certificate.clone(),
        http2: tls.http2,
        ecdh_curve: tls.ecdh_curve.clone(),
        user_agent: tls.user_agent.clone(),
    }
}
