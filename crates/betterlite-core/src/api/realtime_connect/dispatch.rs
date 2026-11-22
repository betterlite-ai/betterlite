use std::error::Error;

use betterlite_config::api::realtime_connect::RealtimeConnectionSettings;
use betterlite_context::resource_config_selection::realtime::select_realtime_config;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::updates::{CallUpdate, update_from_kwargs};
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionContext, ResolutionRequest, ResolvedModel, resolve,
};
use betterlite_openai::realtime::url::QueryParams;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::branches;
use super::branches::DirectProvider;
use super::{
    Branch, ConnectionArgs, ConnectionQuery, GUARDRAILS_KEY, QUERY_MODEL_KEY, RealtimeConnectCall,
    RealtimeConnectDispatch, RealtimeConnectError, RealtimeConnection,
};
use crate::api::realtime_contracts::error::HeaderMap;

// Everything the branches share: the resolved identity, the merged headers, the copied query, the metadata and the logging update.
pub(super) struct Prepared {
    pub(super) model: String,
    pub(super) provider: ProviderIdentity,
    pub(super) dynamic_api_base: Option<String>,
    pub(super) dynamic_api_key: Option<SecretString>,
    pub(super) headers: HeaderMap,
    pub(super) query_params: Option<IndexMap<String, String>>,
    pub(super) metadata: IndexMap<String, Json>,
}

// which optional arguments a branch forwards, since only B2457 and the Vertex transport take them all
pub(super) enum Forwarding {
    Provider,
    Connector,
    Bedrock,
}

pub async fn arealtime<Dispatch>(
    dispatch: &Dispatch,
    settings: &RealtimeConnectionSettings,
    logging: &mut LoggingState,
    resolution: &ResolutionContext<'_>,
    call: &RealtimeConnectCall<'_>,
) -> Result<RealtimeConnection<Dispatch::ProviderConfig>, RealtimeConnectError<Dispatch::Error>>
where
    Dispatch: RealtimeConnectDispatch,
{
    let prepared = prepare(logging, resolution, call)?;
    if let ProviderIdentity::Known(provider) = &prepared.provider
        && let Some(tag) = select_realtime_config(&prepared.model, *provider)
    {
        // generic configs receive the original named api_base and api_key, never the resolver's replacements
        let config = dispatch.provider_config(tag)?;
        let args = connection_args(
            call,
            &prepared,
            call.params.api_base.clone(),
            call.params.api_key.clone(),
            Forwarding::Provider,
        );
        return Ok(RealtimeConnection {
            args,
            branch: Branch::ProviderConfig { config, tag },
        });
    }
    connect(dispatch, settings, call, &prepared).await
}

fn prepare<OperationError>(
    logging: &mut LoggingState,
    resolution: &ResolutionContext<'_>,
    call: &RealtimeConnectCall<'_>,
) -> Result<Prepared, RealtimeConnectError<OperationError>>
where
    OperationError: Error + 'static,
{
    let request = ResolutionRequest {
        model: call.model,
        custom_llm_provider: None,
        api_base: call.params.api_base.as_deref(),
        api_key: call.params.api_key.as_ref(),
    };
    let resolved = resolve(&request, resolution)?;
    let headers = merge_headers(call);
    let query_params = normalized_query(call, &resolved);
    let metadata = build_litellm_metadata(
        call.params.litellm_metadata.as_ref(),
        call.params.metadata.as_ref(),
        call.params.guardrails.as_ref(),
    );
    log_call(logging, call, &resolved)?;
    Ok(Prepared {
        model: resolved.model,
        provider: resolved.provider,
        dynamic_api_base: resolved.api_base,
        dynamic_api_key: resolved.api_key,
        headers,
        query_params,
        metadata,
    })
}

fn merge_headers(call: &RealtimeConnectCall<'_>) -> HeaderMap {
    let mut headers = call.params.headers.clone().unwrap_or_default();
    if let Some(extra) = &call.params.extra_headers {
        headers.extend(extra.clone());
    }
    headers
}

fn normalized_query(
    call: &RealtimeConnectCall<'_>,
    resolved: &ResolvedModel,
) -> Option<IndexMap<String, String>> {
    let mut query_params = call.query_params.cloned();
    if let Some(query) = query_params.as_mut()
        && query.contains_key(QUERY_MODEL_KEY)
    {
        query.insert(String::from(QUERY_MODEL_KEY), resolved.model.clone());
    }
    query_params
}

// LiteLLM merges the kwargs metadata and litellm_metadata for update_from_kwargs and adds the call type to the params
fn log_call<OperationError>(
    logging: &mut LoggingState,
    call: &RealtimeConnectCall<'_>,
    resolved: &ResolvedModel,
) -> Result<(), RealtimeConnectError<OperationError>>
where
    OperationError: Error + 'static,
{
    let mut kwargs: IndexMap<String, Json> = IndexMap::new();
    if let Some(metadata) = &call.params.metadata {
        kwargs.insert(String::from("metadata"), Json::Object(metadata.clone()));
    }
    if let Some(metadata) = &call.params.litellm_metadata {
        kwargs.insert(
            String::from("litellm_metadata"),
            Json::Object(metadata.clone()),
        );
    }
    let mut litellm_params = call.litellm_params.clone();
    litellm_params.insert(String::from("_arealtime"), Json::Bool(true));
    let mut additional_params: IndexMap<String, Json> = IndexMap::new();
    additional_params.insert(
        String::from("custom_llm_provider"),
        Json::Text(resolved.provider.slug().to_owned()),
    );
    Ok(update_from_kwargs(
        logging,
        &kwargs,
        Some(&litellm_params),
        CallUpdate {
            model: Some(resolved.model.clone()),
            user: call.params.user.clone(),
            optional_params: IndexMap::new(),
            additional_params,
        },
    )?)
}

#[must_use]
pub fn build_litellm_metadata(
    litellm_metadata: Option<&IndexMap<String, Json>>,
    metadata: Option<&IndexMap<String, Json>>,
    guardrails: Option<&Json>,
) -> IndexMap<String, Json> {
    let mut built = litellm_metadata.cloned().unwrap_or_default();
    let from_metadata = metadata.and_then(|metadata| metadata.get(GUARDRAILS_KEY));
    let selected = from_metadata
        .filter(|value| !empty_json(value))
        .or_else(|| guardrails.filter(|value| !empty_json(value)));
    if let Some(guardrails) = selected {
        built.insert(String::from(GUARDRAILS_KEY), guardrails.clone());
    }
    built
}

// Python's `or` over the credential and location slots: an empty value falls through
pub(super) fn empty_json(value: &Json) -> bool {
    match value {
        Json::Null => true,
        Json::Text(text) => text.is_empty(),
        Json::List(items) => items.is_empty(),
        Json::Object(fields) => fields.is_empty(),
        Json::Bool(_) | Json::Number(_) => false,
    }
}

async fn connect<Dispatch>(
    dispatch: &Dispatch,
    settings: &RealtimeConnectionSettings,
    call: &RealtimeConnectCall<'_>,
    prepared: &Prepared,
) -> Result<RealtimeConnection<Dispatch::ProviderConfig>, RealtimeConnectError<Dispatch::Error>>
where
    Dispatch: RealtimeConnectDispatch,
{
    match &prepared.provider {
        ProviderIdentity::Known(ProviderId::Azure | ProviderId::AzureAi) => {
            branches::azure_connection(dispatch, settings, call, prepared)
        }
        ProviderIdentity::Known(ProviderId::Openai) => Ok(branches::direct_connection(
            DirectProvider::OpenAi,
            settings,
            call,
            prepared,
        )),
        ProviderIdentity::Known(ProviderId::Bedrock) => {
            Ok(branches::bedrock_connection(call, prepared))
        }
        ProviderIdentity::Known(ProviderId::Xai) => Ok(branches::direct_connection(
            DirectProvider::Xai,
            settings,
            call,
            prepared,
        )),
        ProviderIdentity::Known(ProviderId::VertexAi) => {
            branches::vertex::vertex_connection(settings, dispatch, call, prepared).await
        }
        ProviderIdentity::Known(_unsupported) => Err(unsupported(prepared)),
        ProviderIdentity::Anyscale | ProviderIdentity::Custom(_slug) => Err(unsupported(prepared)),
    }
}

fn unsupported<OperationError>(prepared: &Prepared) -> RealtimeConnectError<OperationError>
where
    OperationError: Error + 'static,
{
    RealtimeConnectError::UnsupportedModel {
        model: prepared.model.clone(),
    }
}

pub(super) fn connection_args(
    call: &RealtimeConnectCall<'_>,
    prepared: &Prepared,
    api_base: Option<String>,
    api_key: Option<SecretString>,
    forwarding: Forwarding,
) -> ConnectionArgs {
    let query_params = match forwarding {
        Forwarding::Provider => ConnectionQuery::Transport(prepared.query_params.clone()),
        Forwarding::Connector => {
            ConnectionQuery::Connector(connector_query(prepared.query_params.as_ref()))
        }
        Forwarding::Bedrock => ConnectionQuery::Omitted,
    };
    let (headers, client) = match forwarding {
        Forwarding::Provider => (Some(prepared.headers.clone()), call.client.clone()),
        Forwarding::Connector | Forwarding::Bedrock => (None, None),
    };
    let (litellm_metadata, user_api_key_dict) = match forwarding {
        Forwarding::Bedrock => (None, None),
        Forwarding::Provider | Forwarding::Connector => (
            Some(prepared.metadata.clone()),
            call.params.user_api_key_dict.clone(),
        ),
    };
    ConnectionArgs {
        model: prepared.model.clone(),
        api_base,
        api_key,
        timeout: call.params.timeout,
        headers,
        client,
        user_api_key_dict,
        litellm_metadata,
        query_params,
    }
}

pub(super) fn connector_query(query: Option<&IndexMap<String, String>>) -> Option<QueryParams> {
    query.map(|query| {
        query
            .iter()
            .map(|(name, value)| (name.clone(), Some(value.clone())))
            .collect()
    })
}
