use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::openai_base_config::CatalogCapabilities;

use crate::operation::{OperationError, OperationRequest, OperationResult};
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedRequest, Upstream};
use crate::provider::Adapter;

mod aleph;
pub mod custom;
pub mod error;
mod proxy;
mod tests;

// The three providers main.py still serves through its legacy dispatch paths; each keeps its own wire protocol.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Legacy {
    #[default]
    LitellmProxy,
    AlephAlpha,
    Custom,
}

impl Legacy {
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::LitellmProxy => "litellm_proxy",
            Self::AlephAlpha => "aleph_alpha",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LegacyRequest;

#[derive(Debug)]
pub struct LegacyResult;

#[derive(Debug, Clone)]
pub struct LegacyPrepared(Call);

#[derive(Debug, Clone)]
enum Call {
    Proxy(proxy::Call),
    Aleph(Box<aleph::Call>),
    Custom(Box<custom::Call>),
}

impl Adapter for Legacy {
    const NAME: &'static str = "legacy";

    fn prepare(
        &self,
        request: &OperationRequest,
        deployment: &Deployment,
        catalog: &CatalogCapabilities,
    ) -> Result<PreparedRequest, OperationError> {
        let call = match self {
            Self::LitellmProxy => Call::Proxy(proxy::prepare(request, deployment, catalog)?),
            Self::AlephAlpha => Call::Aleph(Box::new(aleph::prepare(request, deployment)?)),
            Self::Custom => Call::Custom(Box::new(custom::prepare(request, deployment)?)),
        };
        Ok(PreparedRequest::Legacy(LegacyPrepared(call)))
    }

    async fn execute(
        &self,
        client: &ProxiedClient,
        prepared: &PreparedRequest,
        _deployment: &Deployment,
    ) -> Result<Upstream, OperationError> {
        let PreparedRequest::Legacy(inner) = prepared else {
            return Err(OperationError::UnsupportedOperation {
                provider: String::from(<Self as Adapter>::NAME),
                operation: String::from("provider endpoint"),
            });
        };
        match &inner.0 {
            Call::Proxy(call) => proxy::execute(client, call).await,
            Call::Aleph(call) => aleph::execute(client, call).await,
            Call::Custom(call) => custom::execute(client, call).await,
        }
    }

    fn decode(
        &self,
        prepared: &PreparedRequest,
        upstream: Upstream,
    ) -> Result<OperationResult, OperationError> {
        let PreparedRequest::Legacy(inner) = prepared else {
            return Err(OperationError::UnsupportedOperation {
                provider: String::from(<Self as Adapter>::NAME),
                operation: String::from("provider endpoint"),
            });
        };
        match &inner.0 {
            Call::Proxy(call) => proxy::decode(call, upstream),
            Call::Aleph(call) => aleph::decode(call, upstream),
            Call::Custom(call) => custom::decode(call, upstream),
        }
    }
}
