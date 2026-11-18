use betterlite_models::decisions::Json;

use super::config::ChatConfig;
use super::prepare::Prepared;
use super::send;
use super::send::streams::{StreamFacts, open_stream};
use super::transport::ChatTransport;
use super::{CallFacts, CallStyle, ChatDeps, ChatResult};
use crate::anthropic_http_stream::call::StreamBuild;
use crate::usage::ReasoningTokenCounter;

// The buffered send and the stream open every provider's completion runs once its own prepare returned
pub async fn finish<'call, Config, Client, Counter>(
    facts: CallFacts<'call>,
    prepared: Prepared,
    deps: ChatDeps<'call, Config, Client, Counter>,
) -> ChatResult<Client, Counter>
where
    Config: ChatConfig,
    Client: ChatTransport,
    Counter: ReasoningTokenCounter + Unpin + Send + 'call,
{
    if !prepared.stream {
        return send::buffered(facts, prepared, deps).await;
    }
    let messages = Json::List(facts.messages);
    let stream_facts = StreamFacts {
        model: facts.model,
        api_base: facts.api_base,
        messages: &messages,
        data: prepared.body,
        headers: prepared.header_map,
        timeout: facts.timeout,
        json_mode: prepared.json_mode,
        optional_params: &prepared.optional_params,
        litellm_params: &facts.litellm_params,
        custom_llm_provider: facts.sync_stream.custom_llm_provider,
        names: facts.sync_stream.names,
    };
    let build = StreamBuild {
        ids: deps.ids,
        counter: deps.counter,
        max_event_bytes: deps.max_event_bytes,
        max_error_body_bytes: deps.max_error_body_bytes,
    };
    open_stream(
        deps.clients,
        deps.stream_logger,
        build,
        stream_facts,
        facts.style == CallStyle::Async,
    )
    .await
}
