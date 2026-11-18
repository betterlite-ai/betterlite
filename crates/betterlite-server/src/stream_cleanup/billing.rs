use std::sync::Arc;

use betterlite_logging::fields::usage::parse::usage_from_mapping;
use betterlite_logging::fields::usage::project::usage_json;
use betterlite_logging::logging_streams::cache_usage::backfill_cache_usage;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::log_text::call_id_text;
use super::model::restore_partial_model;
use super::ports::RequestLogging;
use crate::sse_data_generator::types::{CollectedStream, StreamSnapshot};

pub(super) async fn bill_partial(
    logging: &dyn RequestLogging,
    disable_streaming_logging: bool,
    request_data: &JsonObject,
    response: Option<&Arc<dyn StreamSnapshot>>,
) -> bool {
    if disable_streaming_logging {
        return false;
    }
    let Some(details) = logging.call_details(request_data) else {
        return false;
    };
    if details.get("has_dispatched_final_stream_success") == Some(&Json::Bool(true)) {
        return true;
    }
    let Some(snapshot) = response else {
        return false;
    };
    let collected = snapshot.collected();
    if collected.chunks.is_empty() {
        return false;
    }
    tracing::debug!(
        "Billing partial streamed spend for {} chunks after client disconnect, litellm_call_id={}",
        collected.chunks.len(),
        call_id_text(request_data)
    );
    let Some(partial) = assembled_partial(logging, request_data, &collected).await else {
        return false;
    };
    match logging.dispatch_success(request_data, partial).await {
        Ok(()) => true,
        Err(error) => {
            tracing::debug!("Failed to dispatch disconnect billing event: {error}");
            false
        }
    }
}

async fn assembled_partial(
    logging: &dyn RequestLogging,
    request_data: &JsonObject,
    collected: &CollectedStream,
) -> Option<JsonObject> {
    let mut partial = match logging
        .assemble_partial(
            request_data,
            &collected.chunks,
            collected.messages.as_deref(),
        )
        .await
    {
        Ok(Some(partial)) => partial,
        Ok(None) => return None,
        Err(error) => {
            tracing::debug!(
                "Failed to assemble partial streamed response for disconnect billing: {error}"
            );
            return None;
        }
    };
    restore_partial_model(
        &mut partial,
        &collected.chunks,
        collected.model.as_deref(),
        request_data,
    );
    backfill_usage(&mut partial);
    Some(partial)
}

fn backfill_usage(partial: &mut JsonObject) {
    let Some(Json::Object(usage)) = partial.get("usage") else {
        return;
    };
    let Ok(mut parsed) = usage_from_mapping(usage) else {
        return;
    };
    backfill_cache_usage(&mut parsed);
    partial.insert("usage".to_owned(), usage_json(&parsed));
}
