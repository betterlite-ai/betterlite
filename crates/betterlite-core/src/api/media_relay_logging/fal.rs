use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::PriceLookup;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::error::MediaRelayLoggingError;
use super::spend::{MediaSpend, decimal_json};
use crate::api::fal::pricing::{fal_ai_passthrough_cost, fal_ai_queue_base};
use crate::cost::image_dispatch::options::OptionalParams;
use crate::openai_classification::parse_route;

pub const PROVIDER: &str = "fal_ai";

const REQUESTS_SEGMENT: &str = "/requests/";

pub struct FalPassthroughRequest<'request> {
    pub url_route: &'request str,
    pub response_body: &'request Json,
    pub request_body: &'request OptionalParams,
    pub kwargs: &'request PassThroughLoggingKwargs,
    pub lookup: &'request dyn PriceLookup,
    // The configured FAL_AI_QUEUE_API_BASE; absent means the public queue host.
    pub queue_base: Option<&'request str>,
}

pub struct FalPassthroughOutcome {
    pub outcome: HandlerOutcome,
    pub spend: MediaSpend,
}

pub fn fal_ai_passthrough_outcome(
    request: &FalPassthroughRequest<'_>,
) -> Result<FalPassthroughOutcome, MediaRelayLoggingError> {
    let path = upstream_path(request.url_route, &fal_ai_queue_base(request.queue_base));
    let model = model_from_path(&path);
    let response_cost = if path.contains(REQUESTS_SEGMENT) {
        None
    } else {
        fal_ai_passthrough_cost(&model, request.request_body, request.lookup)?
    };
    let response = response_json(image_urls(request.response_body), response_cost);
    let spend = MediaSpend {
        model,
        custom_llm_provider: PROVIDER,
        response_cost,
    };
    let outcome = HandlerOutcome {
        result: Some(PassThroughResponse::Image(response)),
        kwargs: spend.kwargs(request.kwargs),
    };
    Ok(FalPassthroughOutcome { outcome, spend })
}

// A status or result route sits under the submission's /requests/ path, so only it names the model.
fn model_from_path(path: &str) -> String {
    let Some((model, _requests)) = path.split_once(REQUESTS_SEGMENT) else {
        return path.to_owned();
    };
    model.to_owned()
}

// The route may carry the queue host; the model starts after the configured base's own path.
fn upstream_path(url_route: &str, queue_base: &str) -> String {
    let base_path = parse_route(queue_base).path.trim_matches('/').to_owned();
    let raw_path = parse_route(url_route).path.trim_matches('/').to_owned();
    if base_path.is_empty() {
        return raw_path;
    }
    let prefix = format!("{base_path}/");
    raw_path
        .strip_prefix(&prefix)
        .unwrap_or(raw_path.as_str())
        .to_owned()
}

// Every top-level value holding a url: one object or a list of objects, never nested deeper.
fn image_urls(body: &Json) -> Vec<String> {
    let Json::Object(fields) = body else {
        return Vec::new();
    };
    let mut urls = Vec::new();
    for value in fields.values() {
        match value {
            Json::Object(part) => push_url(part, &mut urls),
            Json::List(items) => push_urls(items, &mut urls),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {}
        }
    }
    urls
}

fn push_urls(items: &[Json], urls: &mut Vec<String>) {
    for item in items {
        if let Json::Object(part) = item {
            push_url(part, urls);
        }
    }
}

fn push_url(part: &IndexMap<String, Json>, urls: &mut Vec<String>) {
    if let Some(Json::Text(url)) = part.get("url") {
        urls.push(url.clone());
    }
}

// The logger reads the cost off the response's hidden params, as it does for every pass-through reply.
fn response_json(urls: Vec<String>, response_cost: Option<Decimal>) -> Json {
    let mut hidden = IndexMap::new();
    hidden.insert(
        "response_cost".to_owned(),
        response_cost.map_or(Json::Null, decimal_json),
    );
    let mut response = IndexMap::new();
    response.insert(
        "data".to_owned(),
        Json::List(urls.into_iter().map(image).collect()),
    );
    response.insert("_hidden_params".to_owned(), Json::Object(hidden));
    Json::Object(response)
}

fn image(url: String) -> Json {
    let mut image = IndexMap::new();
    image.insert("url".to_owned(), Json::Text(url));
    Json::Object(image)
}
