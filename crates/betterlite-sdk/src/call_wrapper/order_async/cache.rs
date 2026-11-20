use std::sync::{Arc, PoisonError};

use betterlite_models::decisions::Json;

use super::AsyncRun;
use crate::call_setup::correlation::CallOutcome;
use crate::call_wrapper::cache::CacheMode;
use crate::call_wrapper::error::CallError;

pub(super) async fn lookup<E>(run: &mut AsyncRun<'_, E>) -> Result<Option<CallOutcome>, CallError> {
    let Some(cache) = run.wrapper.cache.as_ref().map(Arc::clone) else {
        return Ok(None);
    };
    let Some(mut found) = cache
        .lookup(&run.request, &run.call_type, CacheMode::Async)
        .await?
    else {
        return Ok(None);
    };
    if found.final_embedding.is_none()
        && let Some(result) = found.cached.take()
    {
        if result.is_converted_stream() {
            mark_stream(run);
        }
        run.cache = Some(found);
        return Ok(Some(result));
    }
    if found.all_embeddings_cached {
        let response = found.final_embedding.take().unwrap_or(Json::Null);
        run.cache = Some(found);
        return Ok(Some(CallOutcome::Response(response)));
    }
    run.cache = Some(found);
    Ok(None)
}

pub(super) fn mark_stream<E>(run: &AsyncRun<'_, E>) {
    let Some(handle) = &run.logging else {
        return;
    };
    let mut state = handle.state.lock().unwrap_or_else(PoisonError::into_inner);
    state.stream = Some(true);
    state
        .model_call_details
        .insert("stream".to_owned(), Json::Bool(true));
}

pub(super) fn apply_preset_cache_key<E>(run: &AsyncRun<'_, E>) {
    let Some(key) = run
        .cache
        .as_ref()
        .and_then(|found| found.preset_cache_key.clone())
    else {
        return;
    };
    let Some(handle) = &run.logging else {
        return;
    };
    let mut state = handle.state.lock().unwrap_or_else(PoisonError::into_inner);
    state
        .litellm_params
        .insert("preset_cache_key".to_owned(), Json::Text(key));
}
