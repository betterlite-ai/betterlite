use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::dispatch::{MessagesDispatchRequest, MessagesExecutionContinuation};
use super::entry::{MessagesEntryRequest, MessagesEntryResult};
use super::error::MessagesExecuteError;
use super::hooks::{
    build_hook_map, cache_stage, pop_named, pop_typed_arguments, provider_from_hook_map,
    resolve_provider, run_pre_request_hooks, sanitize_messages,
};
use super::seams::{InterceptorRequest, MessagesEntrySeams, advisor_requested};
use crate::api::messages_fake_stream::MessagesFakeStream;

// B4115 sets this marker when it converted a streaming web-search request to a non-streaming call
const WEBSEARCH_STREAM_MARKER: &str = "_websearch_interception_converted_stream";

pub enum MessagesEntryOutcome {
    Returned(MessagesEntryResult),
    Dispatch(Box<MessagesExecutionContinuation>),
}

// the request as the hooks left it, plus the fields the later stages read
struct EntryState {
    request: MessagesEntryRequest,
    messages: Vec<Json>,
    named: NamedArguments,
    provider: Option<String>,
    kwargs: IndexMap<String, Json>,
    original_stream: bool,
}

// the named arguments the hook map carries, with the caller's originals as their defaults
struct NamedArguments {
    metadata: Option<Json>,
    stop_sequences: Option<Json>,
    system: Option<Json>,
    temperature: Option<Json>,
    thinking: Option<Json>,
    tool_choice: Option<Json>,
    top_k: Option<Json>,
    top_p: Option<Json>,
    tools: Option<Vec<Json>>,
    stream: Option<bool>,
}

// the request between the cache hook and the named-argument pops
struct PendingHooks {
    request: MessagesEntryRequest,
    messages: Vec<Json>,
    system: Option<Json>,
    kwargs: IndexMap<String, Json>,
    provider: Option<String>,
    original_stream: bool,
    hook_map: IndexMap<String, Json>,
}

// The decorated anthropic_messages body, in LiteLLM's order; the final dispatch comes back as an effect
pub async fn execute(
    request: MessagesEntryRequest,
    seams: &MessagesEntrySeams<'_>,
) -> Result<MessagesEntryOutcome, MessagesExecuteError> {
    let state = prepare(request, seams).await?;
    if let Some(result) = state.search(seams).await? {
        return Ok(MessagesEntryOutcome::Returned(result));
    }
    if let Some(result) = state.intercept(seams).await? {
        return Ok(MessagesEntryOutcome::Returned(result));
    }
    if advisor_requested(state.named.tools.as_deref(), state.provider.as_deref()) {
        return Err(MessagesExecuteError::AdvisorNotBuilt);
    }
    let prepared = state.into_dispatch();
    let continuation =
        MessagesExecutionContinuation::prepare(prepared, Arc::clone(&seams.redactor))?;
    Ok(MessagesEntryOutcome::Dispatch(Box::new(continuation)))
}

async fn prepare(
    request: MessagesEntryRequest,
    seams: &MessagesEntrySeams<'_>,
) -> Result<EntryState, MessagesExecuteError> {
    let pending = hooks_pending(request, seams).await?;
    settle(pending, seams)
}

// The cache hook, the provider resolution and every pre-request hook, up to the named-argument pops
async fn hooks_pending(
    request: MessagesEntryRequest,
    seams: &MessagesEntrySeams<'_>,
) -> Result<PendingHooks, MessagesExecuteError> {
    let mut kwargs = request.kwargs.clone();
    let sanitized = sanitize_messages(&request.messages)?;
    let cached = cache_stage(seams, &request, &sanitized, &mut kwargs)?;
    let requested_stream = request.stream.unwrap_or(false);
    let original_stream =
        requested_stream || matches!(kwargs.get(WEBSEARCH_STREAM_MARKER), Some(Json::Bool(true)));
    let provider = request
        .custom_llm_provider
        .clone()
        .filter(|name| !name.is_empty());
    let hook_provider = provider
        .clone()
        .or_else(|| resolve_provider(&request, seams));
    let hook_map = build_hook_map(
        &request,
        hook_provider.as_deref(),
        requested_stream,
        &kwargs,
    );
    let hook_map = run_pre_request_hooks(
        seams.pre_request_hooks,
        &request.model,
        &cached.messages,
        hook_map,
    )
    .await?;
    Ok(PendingHooks {
        request,
        messages: cached.messages,
        system: cached.system,
        kwargs,
        provider,
        original_stream,
        hook_map,
    })
}

// After the hooks: the typed pops, the provider fallback, and the hook map merged into the extras
fn settle(
    mut pending: PendingHooks,
    seams: &MessagesEntrySeams<'_>,
) -> Result<EntryState, MessagesExecuteError> {
    let named = pop_named_arguments(
        &pending.request,
        &mut pending.hook_map,
        pending.system.take(),
    )?;
    let provider = match pending.provider {
        Some(name) => Some(name),
        None => match provider_from_hook_map(&pending.hook_map)? {
            Some(name) => Some(name),
            None => resolve_provider(&pending.request, seams),
        },
    };
    let _removed = pending.hook_map.shift_remove("litellm_params");
    for (key, value) in pending.hook_map {
        pending.kwargs.insert(key, value);
    }
    Ok(EntryState {
        request: pending.request,
        messages: pending.messages,
        named,
        provider,
        kwargs: pending.kwargs,
        original_stream: pending.original_stream,
    })
}

fn pop_named_arguments(
    request: &MessagesEntryRequest,
    hook_map: &mut IndexMap<String, Json>,
    system: Option<Json>,
) -> Result<NamedArguments, MessagesExecuteError> {
    let requested_stream = request.stream.unwrap_or(false);
    let (tools, stream) =
        pop_typed_arguments(hook_map, request.tools.clone(), Some(requested_stream))?;
    Ok(NamedArguments {
        metadata: pop_named(hook_map, "metadata", request.metadata.clone()),
        stop_sequences: pop_named(hook_map, "stop_sequences", request.stop_sequences.clone()),
        system: pop_named(hook_map, "system", system),
        temperature: pop_named(hook_map, "temperature", request.temperature.clone()),
        thinking: pop_named(hook_map, "thinking", request.thinking.clone()),
        tool_choice: pop_named(hook_map, "tool_choice", request.tool_choice.clone()),
        top_k: pop_named(hook_map, "top_k", request.top_k.clone()),
        top_p: pop_named(hook_map, "top_p", request.top_p.clone()),
        tools,
        stream,
    })
}

impl EntryState {
    async fn search(
        &self,
        seams: &MessagesEntrySeams<'_>,
    ) -> Result<Option<MessagesEntryResult>, MessagesExecuteError> {
        let mut search_kwargs = self.kwargs.clone();
        search_kwargs.insert(
            "metadata".to_owned(),
            self.named.metadata.clone().unwrap_or(Json::Null),
        );
        for searcher in seams.search_short_circuits {
            let response = searcher
                .try_short_circuit_search(
                    &self.request.model,
                    &self.messages,
                    self.named.tools.as_deref(),
                    self.provider.as_deref(),
                    &search_kwargs,
                )
                .await?;
            if let Some(value) = response {
                return Ok(Some(self.search_result(value)?));
            }
        }
        Ok(None)
    }

    fn search_result(&self, value: Json) -> Result<MessagesEntryResult, MessagesExecuteError> {
        if self.original_stream {
            return Ok(MessagesEntryResult::Stream(Box::new(
                MessagesFakeStream::new(&value)?,
            )));
        }
        Ok(MessagesEntryResult::Response(value))
    }

    async fn intercept(
        &self,
        seams: &MessagesEntrySeams<'_>,
    ) -> Result<Option<MessagesEntryResult>, MessagesExecuteError> {
        for interceptor in seams.interceptors {
            if !interceptor.can_handle(self.named.tools.as_deref(), self.provider.as_deref()) {
                continue;
            }
            let result = interceptor
                .handle(&InterceptorRequest {
                    model: &self.request.model,
                    messages: &self.messages,
                    tools: self.named.tools.as_deref(),
                    stream: self.original_stream,
                    max_tokens: self.request.max_tokens,
                    custom_llm_provider: self.provider.as_deref(),
                    api_key: self.request.api_key.as_ref(),
                    api_base: self.request.api_base.as_deref(),
                    metadata: self.named.metadata.as_ref(),
                    stop_sequences: self.named.stop_sequences.as_ref(),
                    system: self.named.system.as_ref(),
                    temperature: self.named.temperature.as_ref(),
                    thinking: self.named.thinking.as_ref(),
                    tool_choice: self.named.tool_choice.as_ref(),
                    top_k: self.named.top_k.as_ref(),
                    top_p: self.named.top_p.as_ref(),
                    kwargs: &self.kwargs,
                })
                .await?;
            return Ok(Some(result));
        }
        Ok(None)
    }

    fn into_dispatch(mut self) -> MessagesDispatchRequest {
        self.kwargs.insert("is_async".to_owned(), Json::Bool(true));
        MessagesDispatchRequest {
            max_tokens: self.request.max_tokens,
            messages: self.messages,
            model: self.request.model,
            metadata: self.named.metadata,
            stop_sequences: self.named.stop_sequences,
            stream: self.named.stream,
            system: self.named.system,
            temperature: self.named.temperature,
            thinking: self.named.thinking,
            tool_choice: self.named.tool_choice,
            tools: self.named.tools,
            top_k: self.named.top_k,
            top_p: self.named.top_p,
            api_key: self.request.api_key,
            api_base: self.request.api_base,
            client: self.request.client,
            custom_llm_provider: self.provider,
            kwargs: self.kwargs,
            presanitized: true,
        }
    }
}
