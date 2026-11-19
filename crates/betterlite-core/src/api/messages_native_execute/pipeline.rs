use std::sync::Arc;

use betterlite_anthropic::message_replay::tool_ids::sanitize_tool_use_ids;
use betterlite_anthropic::message_replay::{BlockStrip, strip_blocks};
use betterlite_anthropic::web_replay::flatten_web_search_results;
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptJson;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use super::dispatch::{MessagesDispatchRequest, MessagesExecutionContinuation};
use super::entry::{MessagesEntryRequest, MessagesEntryResult};
use super::error::MessagesExecuteError;
use super::hooks::{litellm_params, pop_named, pop_stream, pop_tools, provider_from_hook_map};
use super::seams::{
    CacheControlOutcome, CacheControlRequest, InterceptorRequest, MessagesEntrySeams,
    advisor_requested,
};
use crate::api::messages_fake_stream::MessagesFakeStream;
use crate::api::messages_native_transform::error::TransformError;
use crate::api::messages_reasoning_route::seams::ProviderLookup;

// B4115 sets this marker when it converted a streaming web-search request to a non-streaming call
const WEBSEARCH_STREAM_MARKER: &str = "_websearch_interception_converted_stream";

pub enum MessagesEntryOutcome {
    Returned(MessagesEntryResult),
    Dispatch(Box<MessagesExecutionContinuation>),
}

// The decorated anthropic_messages body, in LiteLLM's order; the final dispatch comes back as an effect
pub async fn execute(
    request: MessagesEntryRequest,
    seams: &MessagesEntrySeams<'_>,
) -> Result<MessagesEntryOutcome, MessagesExecuteError> {
    let MessagesEntryRequest {
        max_tokens,
        messages,
        model,
        metadata,
        stop_sequences,
        stream,
        system,
        temperature,
        thinking,
        tool_choice,
        tools,
        top_k,
        top_p,
        api_key,
        api_base,
        client,
        custom_llm_provider,
        mut kwargs,
    } = request;

    let sanitized_messages = sanitize_messages(&messages)?;
    let CacheControlOutcome {
        messages: hook_messages,
        system: hook_system,
    } = seams
        .cache
        .maybe_inject_cache_control(CacheControlRequest {
            messages: &sanitized_messages,
            system: system.as_ref(),
            kwargs: &mut kwargs,
            model: &model,
            custom_llm_provider: custom_llm_provider.as_deref(),
            tools: tools.as_deref(),
            api_base: api_base.as_deref(),
        })?;

    // anthropic_messages defaults stream to False, so an absent value is the default, not None
    let requested_stream = stream.unwrap_or(false);
    let original_stream =
        requested_stream || matches!(kwargs.get(WEBSEARCH_STREAM_MARKER), Some(Json::Bool(true)));

    let mut provider = custom_llm_provider.clone().filter(|name| !name.is_empty());
    let exposed_key = api_key
        .as_ref()
        .map(|secret| secret.expose_secret().as_str());
    let hook_provider = match &provider {
        Some(name) => Some(name.clone()),
        None => resolve_provider(
            &model,
            api_base.as_deref(),
            exposed_key,
            custom_llm_provider.as_deref(),
            seams,
        ),
    };

    let mut hook_map: IndexMap<String, Json> = IndexMap::new();
    hook_map.insert(
        "tools".to_owned(),
        match tools.as_deref() {
            Some(list) => Json::List(list.to_vec()),
            None => Json::Null,
        },
    );
    hook_map.insert("stream".to_owned(), Json::Bool(requested_stream));
    hook_map.insert(
        "litellm_params".to_owned(),
        litellm_params(hook_provider.as_deref()),
    );
    hook_map.insert(
        "tool_choice".to_owned(),
        tool_choice.clone().unwrap_or(Json::Null),
    );
    for (key, value) in &kwargs {
        hook_map.insert(key.clone(), value.clone());
    }

    for hook in seams.pre_request_hooks {
        let modified = hook
            .pre_request(&model, &hook_messages, &mut hook_map)
            .await?;
        if let Some(replacement) = modified {
            hook_map = replacement;
        }
    }

    let final_tools = pop_tools(&mut hook_map, tools)?;
    let final_stream = pop_stream(&mut hook_map, Some(requested_stream))?;
    let final_metadata = pop_named(&mut hook_map, "metadata", metadata);
    let final_stop_sequences = pop_named(&mut hook_map, "stop_sequences", stop_sequences);
    let final_system = pop_named(&mut hook_map, "system", hook_system);
    let final_temperature = pop_named(&mut hook_map, "temperature", temperature);
    let final_thinking = pop_named(&mut hook_map, "thinking", thinking);
    let final_tool_choice = pop_named(&mut hook_map, "tool_choice", tool_choice);
    let final_top_k = pop_named(&mut hook_map, "top_k", top_k);
    let final_top_p = pop_named(&mut hook_map, "top_p", top_p);

    if provider.is_none() {
        let from_hook = provider_from_hook_map(&hook_map)?;
        provider = match from_hook {
            Some(name) => Some(name),
            None => resolve_provider(
                &model,
                api_base.as_deref(),
                exposed_key,
                custom_llm_provider.as_deref(),
                seams,
            ),
        };
    }
    let _removed = hook_map.shift_remove("litellm_params");
    for (key, value) in hook_map {
        kwargs.insert(key, value);
    }

    let mut search_kwargs = kwargs.clone();
    search_kwargs.insert(
        "metadata".to_owned(),
        final_metadata.clone().unwrap_or(Json::Null),
    );
    for searcher in seams.search_short_circuits {
        let response = searcher
            .try_short_circuit_search(
                &model,
                &hook_messages,
                final_tools.as_deref(),
                provider.as_deref(),
                &search_kwargs,
            )
            .await?;
        if let Some(value) = response {
            let result = if original_stream {
                MessagesEntryResult::Stream(Box::new(MessagesFakeStream::new(&value)?))
            } else {
                MessagesEntryResult::Response(value)
            };
            return Ok(MessagesEntryOutcome::Returned(result));
        }
    }

    for interceptor in seams.interceptors {
        if !interceptor.can_handle(final_tools.as_deref(), provider.as_deref()) {
            continue;
        }
        let result = interceptor
            .handle(&InterceptorRequest {
                model: &model,
                messages: &hook_messages,
                tools: final_tools.as_deref(),
                stream: original_stream,
                max_tokens,
                custom_llm_provider: provider.as_deref(),
                api_key: api_key.as_ref(),
                api_base: api_base.as_deref(),
                metadata: final_metadata.as_ref(),
                stop_sequences: final_stop_sequences.as_ref(),
                system: final_system.as_ref(),
                temperature: final_temperature.as_ref(),
                thinking: final_thinking.as_ref(),
                tool_choice: final_tool_choice.as_ref(),
                top_k: final_top_k.as_ref(),
                top_p: final_top_p.as_ref(),
                kwargs: &kwargs,
            })
            .await?;
        return Ok(MessagesEntryOutcome::Returned(result));
    }
    if advisor_requested(final_tools.as_deref(), provider.as_deref()) {
        return Err(MessagesExecuteError::AdvisorNotBuilt);
    }

    kwargs.insert("is_async".to_owned(), Json::Bool(true));
    let prepared = MessagesDispatchRequest {
        max_tokens,
        messages: hook_messages,
        model,
        metadata: final_metadata,
        stop_sequences: final_stop_sequences,
        stream: final_stream,
        system: final_system,
        temperature: final_temperature,
        thinking: final_thinking,
        tool_choice: final_tool_choice,
        tools: final_tools,
        top_k: final_top_k,
        top_p: final_top_p,
        api_key,
        api_base,
        client,
        custom_llm_provider: provider,
        kwargs,
        presanitized: true,
    };
    let continuation =
        MessagesExecutionContinuation::prepare(prepared, Arc::clone(&seams.redactor))?;
    Ok(MessagesEntryOutcome::Dispatch(Box::new(continuation)))
}

// B571, then B572, over the prompt types their implementations own
fn sanitize_messages(messages: &[Json]) -> Result<Vec<Json>, MessagesExecuteError> {
    let encoded = serde_json::to_vec(messages).map_err(TransformError::from)?;
    let prompt_messages: Vec<PromptJson> =
        serde_json::from_slice(&encoded).map_err(TransformError::from)?;
    let stripped = strip_blocks(&prompt_messages, BlockStrip::EmptyContent);
    let with_ids = sanitize_tool_use_ids(&stripped);
    let flattened = flatten_web_search_results(&with_ids);
    let reencoded = serde_json::to_vec(&flattened).map_err(TransformError::from)?;
    let contract: Vec<Json> = serde_json::from_slice(&reencoded).map_err(TransformError::from)?;
    Ok(contract)
}

fn resolve_provider(
    model: &str,
    api_base: Option<&str>,
    api_key: Option<&str>,
    custom_llm_provider: Option<&str>,
    seams: &MessagesEntrySeams<'_>,
) -> Option<String> {
    let lookup = ProviderLookup {
        model: Some(model),
        custom_llm_provider,
        api_base,
        api_key,
    };
    seams
        .provider_resolution
        .resolve(&lookup)
        .map(|target| target.provider)
}
