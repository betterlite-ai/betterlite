use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_core::api::responses_agentic_execution::executor::{
    AnthropicContinuation, ContinuationExecutor, ResponsesContinuation,
};
use betterlite_core::api::responses_compact_http::service::{ClientSource, SharedSession};
use betterlite_core::api::responses_create::bridge::{BridgeCall, ChatBridge, Created};
use betterlite_core::api::responses_create::error::BridgeError;
use betterlite_core::api::responses_create::interception::{
    CreateInterception, DispatchArgs, FileSearchArgs, Interception, InterceptionResult,
};
use betterlite_core::api::responses_create_http::config::Params;
use betterlite_core::api::responses_hook_policy::callbacks::{
    DeploymentHookDispatcher, NamedCallbackResolver, PolicyCallback,
};
use betterlite_core::api::responses_provider_selection::resolution::ModelResolver;
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionError, ResolutionRequest, ResolvedModel,
};
use serde_json::value::RawValue;

use super::FixtureError;

pub(crate) struct Outbound {
    client: ProxiedClient,
}

// no environment reaches the fixture, so the policy resolves without a proxy route
pub(crate) fn outbound() -> Result<Outbound, FixtureError> {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())?;
    let client = ProxiedClient::outbound(&Arc::new(policy))?;
    Ok(Outbound { client })
}

impl ClientSource for Outbound {
    type Client = ProxiedClient;

    fn client(
        &self,
        _ssl_verify: Option<bool>,
        _shared_session: Option<SharedSession>,
    ) -> Result<ProxiedClient, ClientError> {
        Ok(self.client.clone())
    }
}

// the dispatch resolves only openai: a declared provider wins, otherwise the model prefix names it
pub(crate) struct DispatchResolver;

impl ModelResolver for DispatchResolver {
    fn resolve_model(
        &self,
        request: &ResolutionRequest<'_>,
    ) -> Result<ResolvedModel, ResolutionError> {
        let (prefix, rest) = request
            .model
            .split_once('/')
            .unwrap_or((request.model, request.model));
        let provider = request.custom_llm_provider.unwrap_or(prefix);
        if provider != "openai" {
            return Err(ResolutionError::MissingProvider {
                model: request.model.to_owned(),
            });
        }
        Ok(ResolvedModel {
            model: rest.to_owned(),
            provider: ProviderIdentity::Custom(String::from("openai")),
            api_base: None,
            api_key: None,
        })
    }
}

pub(crate) struct Nothing;

#[async_trait]
impl DeploymentHookDispatcher for Nothing {
    type Error = io::Error;

    async fn pre_call_deployment_hook(
        &self,
        _request: Params,
        _call_type: &str,
    ) -> Result<Option<Params>, Self::Error> {
        Ok(None)
    }
}

impl NamedCallbackResolver for Nothing {
    fn resolve(&self, _name: &str) -> Option<PolicyCallback> {
        None
    }
}

#[async_trait]
impl ContinuationExecutor for Nothing {
    type Error = io::Error;

    async fn responses(&self, _call: &ResponsesContinuation) -> Result<Box<RawValue>, Self::Error> {
        Err(io::Error::other("a continuation is not expected"))
    }

    async fn anthropic_messages(
        &self,
        _call: &AnthropicContinuation,
    ) -> Result<Box<RawValue>, Self::Error> {
        Err(io::Error::other("a continuation is not expected"))
    }
}

pub(crate) struct NoInterception;

#[async_trait]
impl CreateInterception<()> for NoInterception {
    fn mcp_requested(&self, _tools: Option<&[JsonValue]>) -> bool {
        false
    }

    async fn mcp(&self, _args: &DispatchArgs<'_>, context: ()) -> InterceptionResult<()> {
        Ok(Interception::Continue(context))
    }

    async fn file_search(&self, _args: &FileSearchArgs<'_>, context: ()) -> InterceptionResult<()> {
        Ok(Interception::Continue(context))
    }
}

pub(crate) struct NoBridge;

#[async_trait]
impl ChatBridge<()> for NoBridge {
    async fn respond(
        &self,
        _call: &BridgeCall<'_>,
        _context: (),
    ) -> Result<Created<()>, BridgeError> {
        Err(BridgeError {
            message: String::from("a managed response.create turn does not bridge"),
        })
    }
}
