use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::ResponsesStreamHooks;
use super::ports::{CACHE_WRITE_TASK, CacheWrite, Dispatch, PortFailure};
use crate::api::response_synthetic_events::model_dump::dump_json_safely;
use crate::api::responses_stream_primitives::state::ResponsesStreamBaseState;
use crate::api::responses_stream_primitives::tasks::spawn_logging_task;
use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};

#[must_use]
pub fn converted_stream_requested(kwargs: &IndexMap<String, Json>) -> bool {
    [
        "_websearch_interception_converted_stream",
        "_compression_interception_converted_stream",
        "_headroom_interception_converted_stream",
        "_code_interpreter_interception_converted_stream",
    ]
    .iter()
    .any(|key| value_is_set(kwargs.get(*key)))
}

pub(crate) fn value_is_set(value: Option<&Json>) -> bool {
    match value {
        None | Some(Json::Null | Json::Bool(false)) => false,
        Some(Json::Number(number)) => number
            .to_string()
            .bytes()
            .take_while(|byte| !matches!(byte, b'e' | b'E'))
            .any(|byte| matches!(byte, b'1'..=b'9')),
        Some(Json::Text(text)) => !text.is_empty(),
        Some(Json::List(items)) => !items.is_empty(),
        Some(Json::Object(fields)) => !fields.is_empty(),
        Some(Json::Bool(true)) => true,
    }
}

#[must_use]
pub fn cache_request_kwargs(
    kwargs: &IndexMap<String, Json>,
    preset_cache_key: Option<&str>,
) -> IndexMap<String, Json> {
    let mut copied = kwargs.clone();
    copied.shift_remove("custom_llm_provider");
    if matches!(copied.get("metadata"), Some(Json::Null)) {
        copied.shift_remove("metadata");
    }
    let request_key = copied.shift_remove("cache_key");
    let selected_key = preset_cache_key.map_or_else(
        || request_key.filter(|key| !matches!(key, Json::Null)),
        |preset| Some(Json::Text(preset.to_owned())),
    );
    if let Some(key) = selected_key {
        copied.insert("cache_key".to_owned(), key);
    }
    copied
}

pub fn persist_cache(
    hooks: &ResponsesStreamHooks<'_>,
    state: &mut ResponsesStreamBaseState,
    dispatch: Dispatch,
) -> Result<(), PortFailure> {
    let Some(prepared) = prepare_cache_write(hooks, state)? else {
        return Ok(());
    };
    let Some(cache) = hooks.cache.as_ref().map(Arc::clone) else {
        return Ok(());
    };
    match dispatch {
        Dispatch::Inline => {
            cache.add_cache(prepared)?;
            state.completed_response_cached = true;
        }
        Dispatch::Spawned => {
            let work = async move { cache.async_add_cache(prepared).await };
            state.completed_response_cached =
                spawn_logging_task(&state.pending_tasks, CACHE_WRITE_TASK, work).is_some();
        }
    }
    Ok(())
}

fn prepare_cache_write(
    hooks: &ResponsesStreamHooks<'_>,
    state: &ResponsesStreamBaseState,
) -> Result<Option<CacheWrite>, PortFailure> {
    if state.completed_response_cached {
        return Ok(None);
    }
    let Some(event) = state.completed_response.as_ref() else {
        return Ok(None);
    };
    let ResponsesAPIStreamingResponse::Known(known) = event else {
        return Ok(None);
    };
    let KnownResponseEvent::ResponseCompleted(record) = known.as_ref() else {
        return Ok(None);
    };
    let Some(handler) = hooks.cache_handler.as_ref() else {
        return Ok(None);
    };
    let Some(request_kwargs) = handler.request_kwargs() else {
        return Ok(None);
    };
    if !matches!(request_kwargs.get("stream"), Some(Json::Bool(true)))
        && !converted_stream_requested(request_kwargs)
    {
        return Ok(None);
    }
    let prepared_kwargs = cache_request_kwargs(request_kwargs, handler.preset_cache_key());
    if !handler.should_store_result_in_cache(handler.original_function(), &prepared_kwargs)? {
        return Ok(None);
    }
    if hooks.cache.is_none() {
        return Ok(None);
    }
    let Some(payload) = dump_json_safely(&record.response) else {
        return Ok(None);
    };
    Ok(Some(CacheWrite {
        payload,
        request_kwargs: prepared_kwargs,
        dynamic_cache: handler.dual_cache(),
    }))
}
