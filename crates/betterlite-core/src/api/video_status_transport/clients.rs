use std::str::FromStr as _;

use async_trait::async_trait;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::pool::PoolError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;

use super::error::VideoStatusError;
use super::records::{Params, StatusRequest};

// LiteLLM's HTTPHandler: the blocking send a status call makes
pub trait SyncClient: Send + Sync {
    fn send(&self, request: &StatusRequest) -> Result<UpstreamResponse, TransportError>;
}

// LiteLLM's AsyncHTTPHandler
#[async_trait]
pub trait AsyncClient: Send + Sync {
    async fn send(&self, request: &StatusRequest) -> Result<UpstreamResponse, TransportError>;
}

// B173 behind this: the blocking and async clients are different types, as HTTPHandler and AsyncHTTPHandler are
pub trait ClientFactories: Send + Sync {
    type Synchronous: SyncClient;
    type Asynchronous: AsyncClient;

    fn sync_client(&self, ssl_verify: Option<bool>) -> Result<Self::Synchronous, PoolError>;

    fn async_client(
        &self,
        provider: ProviderId,
        ssl_verify: Option<bool>,
    ) -> Result<Self::Asynchronous, PoolError>;
}

// The client a call sends with: the caller's own, or one the factories built for it
pub enum Selected<'client, Client> {
    Supplied(&'client Client),
    Created(Client),
}

impl<Client> Selected<'_, Client> {
    #[must_use]
    pub const fn get(&self) -> &Client {
        match self {
            Self::Supplied(client) => client,
            Self::Created(client) => client,
        }
    }
}

// litellm_params ssl_verify: absent or null is the default, and any other type is refused
pub(crate) fn ssl_verify(litellm_params: &Params) -> Result<Option<bool>, VideoStatusError> {
    match litellm_params.get("ssl_verify") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(VideoStatusError::FieldType {
            field: "ssl_verify",
            expected: "a boolean",
        }),
    }
}

// litellm.LlmProviders(custom_llm_provider): an unknown name fails where the async client is asked for
pub(crate) fn provider_id(provider: &str) -> Result<ProviderId, VideoStatusError> {
    ProviderId::from_str(provider).map_err(|_unknown| VideoStatusError::UnknownProvider {
        provider: provider.to_owned(),
    })
}
