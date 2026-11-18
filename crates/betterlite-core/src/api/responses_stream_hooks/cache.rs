use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::ResponsesStreamHooks;
use super::events::completed_response;
use super::ports::{CACHE_WRITE_TASK, CacheWrite, PortFailure};
use crate::api::response_synthetic_events::model_dump::dump_json_safely;
use crate::api::responses_stream_primitives::state::ResponsesStreamBaseState;
use crate::api::responses_stream_primitives::tasks::spawn_logging_task;

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
        Some(Json::Number(numeric_value)) => {
            numeric_value.as_i64().is_some_and(|integer| integer != 0)
                || numeric_value.as_u64().is_some_and(|integer| integer != 0)
                || numeric_value
                    .as_f64()
                    .is_some_and(|float_value| float_value != 0.0_f64)
        }
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
    let selected_key = match preset_cache_key {
        Some(preset) => Some(Json::Text(preset.to_owned())),
        None => request_key.filter(|key| !matches!(key, Json::Null)),
    };
    if let Some(key) = selected_key {
        copied.insert("cache_key".to_owned(), key);
    }
    copied
}

impl ResponsesStreamHooks<'_> {
    pub fn persist_cache(&self, state: &mut ResponsesStreamBaseState) -> Result<(), PortFailure> {
        let Some(prepared) = self.prepare_cache_write(state)? else {
            return Ok(());
        };
        let Some(cache) = self.cache.as_ref() else {
            return Ok(());
        };
        cache.add_cache(prepared)?;
        state.completed_response_cached = true;
        Ok(())
    }

    // the is_async=True arm: schedule cache.async_add_cache and mark the response cached once it is scheduled
    pub fn schedule_cache(&self, state: &mut ResponsesStreamBaseState) -> Result<(), PortFailure> {
        let Some(prepared) = self.prepare_cache_write(state)? else {
            return Ok(());
        };
        let Some(cache) = self.cache.as_ref().map(Arc::clone) else {
            return Ok(());
        };
        let work = async move { cache.async_add_cache(prepared).await };
        if spawn_logging_task(&state.pending_tasks, CACHE_WRITE_TASK, work).is_some() {
            state.completed_response_cached = true;
        }
        Ok(())
    }

    fn prepare_cache_write(
        &self,
        state: &ResponsesStreamBaseState,
    ) -> Result<Option<CacheWrite>, PortFailure> {
        if state.completed_response_cached {
            return Ok(None);
        }
        let Some(event) = state.completed_response.as_ref() else {
            return Ok(None);
        };
        let Some(response) = completed_response(event) else {
            return Ok(None);
        };
        let Some(handler) = self.cache_handler.as_ref() else {
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
        let request_kwargs = cache_request_kwargs(request_kwargs, handler.preset_cache_key());
        if !handler.should_store_result_in_cache(handler.original_function(), &request_kwargs)? {
            return Ok(None);
        }
        if self.cache.is_none() {
            return Ok(None);
        }
        let Some(payload) = dump_json_safely(response) else {
            return Ok(None);
        };
        Ok(Some(CacheWrite {
            payload,
            request_kwargs,
            dynamic_cache: handler.dual_cache(),
        }))
    }
}
