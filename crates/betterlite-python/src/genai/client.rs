use super::error::SessionError;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::genai_http::service::ClientSource;
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::pool::PoolError;
use betterlite_prompts::arguments::json::Json;
use std::collections::BTreeMap;
use std::sync::Arc;
mod tests;
// The clients one session hands its calls: built from the environment's proxy, and rebuilt only when a deployment turns verification off.
pub(crate) struct NativeClients {
    environment: BTreeMap<String, String>,
    policy: Arc<ProxyPolicy>,
    pub(crate) default: ProxiedClient,
}
impl NativeClients {
    pub(crate) fn build(environment: &BTreeMap<String, String>) -> Result<Self, SessionError> {
        let trust = ProxyTrust::default();
        let resolved = ProxyPolicy::resolve(&trust, environment)
            .map_err(|source| SessionError::Proxy { source })?;
        let policy = Arc::new(resolved);
        let default =
            ProxiedClient::outbound(&policy).map_err(|source| SessionError::Client { source })?;
        Ok(Self {
            environment: environment.clone(),
            policy,
            default,
        })
    }
    // httpx's verify=False and verify="<path>" build a client of their own; every other value keeps the session's
    fn verified_with(&self, verification: Verification) -> Result<ProxiedClient, ClientError> {
        let ca_file = match &verification {
            Verification::CaFile(path) => Some(path.clone()),
            Verification::Enabled | Verification::Disabled => None,
        };
        let tls = TlsPolicy {
            verification,
            ca_file,
            certificate: None,
            http2: false,
            ecdh_curve: None,
            user_agent: user_agent(&self.environment),
        };
        ProxiedClient::outbound_with_tls(&self.policy, &tls)
            .map_err(|source| ClientError::Pool(PoolError::Client(source)))
    }
}
impl ClientSource for NativeClients {
    type Client = ProxiedClient;
    fn client(&self, ssl_verify: Option<&Json>) -> Result<ProxiedClient, ClientError> {
        match ssl_verify {
            Some(Json::Bool(false)) => self.verified_with(Verification::Disabled),
            Some(Json::Text(path)) => self.verified_with(Verification::CaFile(path.clone())),
            None
            | Some(
                Json::Null | Json::Bool(true) | Json::Number(_) | Json::List(_) | Json::Object(_),
            ) => Ok(self.default.clone()),
        }
    }
}
// TlsPolicy::resolve reads LITELLM_USER_AGENT and falls back to the crate's own name
fn user_agent(environment: &BTreeMap<String, String>) -> String {
    environment
        .get("LITELLM_USER_AGENT")
        .cloned()
        .unwrap_or_else(|| format!("betterlite/{}", env!("CARGO_PKG_VERSION")))
}
