use betterlite_config::tls::{TlsPolicy, Verification};
use betterlite_core::api::genai_http::service::ClientSource;
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::pool::PoolError;
use betterlite_prompts::arguments::json::Json;

// The clients one session hands its calls: built from the environment's proxy, and rebuilt only when a deployment turns verification off
pub struct NativeClients {
    pub(super) default: ProxiedClient,
    user_agent: String,
}

impl NativeClients {
    #[must_use]
    pub const fn new(default: ProxiedClient, user_agent: String) -> Self {
        Self {
            default,
            user_agent,
        }
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
            user_agent: self.user_agent.clone(),
        };
        ProxiedClient::outbound_with_tls(&self.default.policy, &tls)
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
