use betterlite_http::interactions::agents_create_read::AgentsApiConfig;
use betterlite_http::interactions::agents_create_read::list_async::async_list_agents;
use betterlite_http::interactions::agents_create_read::list_sync::list_agents;
use betterlite_models::decisions::Json;

use super::errors::{failure, identity_of};
use super::preparation::{prepare_listing, provider_of};
use super::{
    AgentAnswer, AgentCollection, AgentCollectionError, AgentCollectionHandled,
    AgentCollectionResult, AgentFailure, AgentListing, DEFAULT_PROVIDER, LIST_ASYNC_FLAG,
};

pub fn list<'call, Config>(
    collection: AgentCollection<'call, Config>,
    call: &AgentListing<'_>,
) -> AgentAnswer<'call, Config::ListOutput, Config::Error>
where
    Config: AgentsApiConfig,
    Config::Error: AgentFailure,
{
    let provider = provider_of(call.custom_llm_provider, call.kwargs).map_err(|error| {
        let named = call.custom_llm_provider.unwrap_or(DEFAULT_PROVIDER);
        failure(
            AgentCollectionError::Prepare(error),
            &identity_of("", named, false),
            collection.redactor,
        )
    })?;
    let identity = identity_of("", provider, false);
    let prepared = prepare_listing(collection.logging, call, provider).map_err(|error| {
        failure(
            AgentCollectionError::Prepare(error),
            &identity,
            collection.redactor,
        )
    })?;
    let AgentCollection {
        config,
        clients,
        records,
        redactor,
        ..
    } = collection;
    if prepared.is_async {
        return Ok(AgentCollectionHandled::Pending(Box::pin(async move {
            let request = prepared.request();
            async_list_agents(config, clients, records, None, &request)
                .await
                .map_err(|error| failure(AgentCollectionError::Request(error), &identity, redactor))
        })));
    }
    let request = prepared.request();
    list_agents(config, clients, records, None, &request)
        .map_err(|error| failure(AgentCollectionError::Request(error), &identity, redactor))
        .map(AgentCollectionHandled::Done)
}

pub async fn alist<Config>(
    collection: AgentCollection<'_, Config>,
    call: AgentListing<'_>,
) -> AgentCollectionResult<Config::ListOutput, Config::Error>
where
    Config: AgentsApiConfig,
    Config::ListOutput: Send,
    Config::Error: AgentFailure,
{
    let mut kwargs = call.kwargs.clone();
    kwargs.insert(LIST_ASYNC_FLAG.to_owned(), Json::Bool(true));
    let provider = call.custom_llm_provider.unwrap_or(DEFAULT_PROVIDER);
    let delegation = AgentListing {
        custom_llm_provider: Some(provider),
        extra_headers: call.extra_headers,
        timeout: call.timeout,
        kwargs: &kwargs,
    };
    match list(collection, &delegation)? {
        AgentCollectionHandled::Done(output) => Ok(output),
        AgentCollectionHandled::Pending(pending) => pending.await,
    }
}
