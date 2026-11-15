use betterlite_http::interactions::agents_create_read::AgentsApiConfig;
use betterlite_http::interactions::agents_create_read::create_async::async_create_agent;
use betterlite_http::interactions::agents_create_read::create_sync::create_agent;
use betterlite_models::decisions::Json;

use super::errors::{failure, identity_of};
use super::preparation::{prepare_creation, provider_of};
use super::{
    AgentAnswer, AgentCollection, AgentCollectionError, AgentCollectionHandled,
    AgentCollectionResult, AgentCreation, AgentFailure, CREATE_ASYNC_FLAG, DEFAULT_PROVIDER,
};

pub fn create<'call, Config>(
    collection: AgentCollection<'call, Config>,
    call: &AgentCreation<'_>,
) -> AgentAnswer<'call, Config::CreateOutput, Config::Error>
where
    Config: AgentsApiConfig,
    Config::Error: AgentFailure,
{
    let provider = provider_of(call.custom_llm_provider, call.kwargs).map_err(|error| {
        let named = call.custom_llm_provider.unwrap_or(DEFAULT_PROVIDER);
        failure(
            AgentCollectionError::Prepare(error),
            &identity_of(call.name, named, true),
            collection.redactor,
        )
    })?;
    let identity = identity_of(call.name, provider, true);
    let prepared = prepare_creation(collection.logging, call, provider).map_err(|error| {
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
        request_timeout,
        ..
    } = collection;
    if prepared.is_async {
        return Ok(AgentCollectionHandled::Pending(Box::pin(async move {
            let request = prepared.request(request_timeout);
            async_create_agent(config, clients, records, None, &request)
                .await
                .map_err(|error| failure(AgentCollectionError::Request(error), &identity, redactor))
        })));
    }
    let request = prepared.request(request_timeout);
    create_agent(config, clients, records, None, &request)
        .map_err(|error| failure(AgentCollectionError::Request(error), &identity, redactor))
        .map(AgentCollectionHandled::Done)
}

pub async fn acreate<Config>(
    collection: AgentCollection<'_, Config>,
    call: AgentCreation<'_>,
) -> AgentCollectionResult<Config::CreateOutput, Config::Error>
where
    Config: AgentsApiConfig,
    Config::CreateOutput: Send,
    Config::Error: AgentFailure,
{
    let mut kwargs = call.kwargs.clone();
    kwargs.insert(CREATE_ASYNC_FLAG.to_owned(), Json::Bool(true));
    let provider = call.custom_llm_provider.unwrap_or(DEFAULT_PROVIDER);
    let delegation = AgentCreation {
        name: call.name,
        base_agent: call.base_agent,
        instructions: call.instructions,
        base_environment: call.base_environment,
        custom_llm_provider: Some(provider),
        extra_headers: call.extra_headers,
        extra_body: call.extra_body,
        timeout: call.timeout,
        kwargs: &kwargs,
    };
    match create(collection, &delegation)? {
        AgentCollectionHandled::Done(output) => Ok(output),
        AgentCollectionHandled::Pending(pending) => pending.await,
    }
}
