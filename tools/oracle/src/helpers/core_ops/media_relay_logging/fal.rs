use betterlite_core::api::media_relay_logging::fal::{
    FalPassthroughRequest, fal_ai_passthrough_outcome,
};
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use serde_json::{Map, Value, json};

use super::services::fal_lookup;
use super::{FalCall, failure, relay_answer};

// The gathered image urls, then what every relay handler leaves for the spend logger.
pub(super) fn fal_answer(call: &FalCall) -> Result<Value, Box<HelperError>> {
    let lookup = fal_lookup(&call.catalog)?;
    let kwargs = PassThroughLoggingKwargs {
        entries: call.kwargs.clone(),
    };
    let passthrough = fal_ai_passthrough_outcome(&FalPassthroughRequest {
        url_route: &call.url_route,
        response_body: &call.response_body,
        request_body: &call.request_body,
        kwargs: &kwargs,
        lookup: &lookup,
        queue_base: call.queue_base.as_deref(),
    })
    .map_err(failure)?;
    let mut answer = Map::new();
    answer.insert("urls".to_owned(), json!(image_urls(&passthrough.outcome)));
    Ok(relay_answer(
        answer,
        &passthrough.outcome,
        &passthrough.spend,
    ))
}

fn image_urls(outcome: &HandlerOutcome) -> Vec<String> {
    let Some(PassThroughResponse::Image(Json::Object(fields))) = &outcome.result else {
        return Vec::new();
    };
    let Some(Json::List(items)) = fields.get("data") else {
        return Vec::new();
    };
    let mut urls = Vec::new();
    for item in items {
        if let Json::Object(part) = item
            && let Some(Json::Text(url)) = part.get("url")
        {
            urls.push(url.clone());
        }
    }
    urls
}
