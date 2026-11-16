use std::collections::BTreeMap;

use betterlite_core::api::media_relay_logging::deepgram::{
    DeepgramListenRequest, deepgram_listen_outcome,
};
use betterlite_core::api::media_relay_logging::fal::{
    FalPassthroughRequest, fal_ai_passthrough_outcome,
};
use betterlite_core::api::media_relay_logging::routes::{
    is_deepgram_listen_route, is_fal_ai_route,
};
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    deepgram_routes: Vec<String>,
    #[serde(default)]
    fal_routes: Vec<FalRoute>,
    #[serde(default)]
    deepgram_listens: Vec<DeepgramCall>,
    #[serde(default)]
    fal_passthroughs: Vec<FalCall>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FalRoute {
    url_route: String,
    custom_llm_provider: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeepgramCall {
    upstream_url: String,
    messages: Vec<Json>,
    #[serde(default)]
    kwargs: IndexMap<String, Json>,
    #[serde(default)]
    catalog: IndexMap<String, Json>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FalCall {
    url_route: String,
    response_body: Json,
    #[serde(default)]
    request_body: IndexMap<String, Json>,
    #[serde(default)]
    kwargs: IndexMap<String, Json>,
    #[serde(default)]
    catalog: IndexMap<String, Json>,
    #[serde(default)]
    queue_base: Option<String>,
}

// The case's catalog stands in for litellm.model_cost, so a raw lookup is the key and an effective one is provider-qualified.
struct CaseLookup {
    entries: BTreeMap<String, PriceRecord>,
}

impl PriceLookup for CaseLookup {
    fn effective_model_info(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        let scoped =
            custom_llm_provider.and_then(|name| self.entries.get(&format!("{name}/{model}")));
        scoped.or_else(|| self.entries.get(model)).cloned()
    }

    fn raw_cost_map_entry(&self, key: &str) -> Option<PriceRecord> {
        self.entries.get(key).cloned()
    }

    fn effective_priced_model(
        &self,
        model: &str,
        custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        let record = self.effective_model_info(model, custom_llm_provider)?;
        Some(PricedModel {
            record,
            tiered_pricing: Vec::new(),
            off_peak_pricing: None,
            off_peak_cache_read_present: false,
        })
    }
}

struct Registry {
    catalog: ModelCatalog,
    lookup: CaseLookup,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let deepgram_routes: Vec<bool> = input
            .deepgram_routes
            .iter()
            .map(|route| is_deepgram_listen_route(route))
            .collect();
        let fal_routes: Vec<bool> = input
            .fal_routes
            .iter()
            .map(|route| is_fal_ai_route(&route.url_route, route.custom_llm_provider.as_deref()))
            .collect();
        let mut deepgram_listens = Vec::new();
        for call in &input.deepgram_listens {
            deepgram_listens.push(deepgram_answer(call)?);
        }
        let mut fal_passthroughs = Vec::new();
        for call in &input.fal_passthroughs {
            fal_passthroughs.push(fal_answer(call)?);
        }
        Ok(json!({
            "deepgram_routes": deepgram_routes,
            "fal_routes": fal_routes,
            "deepgram_listens": deepgram_listens,
            "fal_passthroughs": fal_passthroughs,
        }))
    }
}

fn registry(entries: &IndexMap<String, Json>) -> Result<Registry, Box<HelperError>> {
    let bytes = serde_json::to_vec(entries).map_err(failure)?;
    let catalog = ModelCatalog::parse(&bytes, UnknownFields::Reject).map_err(failure)?;
    let lookup = CaseLookup {
        entries: serde_json::from_slice(&bytes).map_err(failure)?,
    };
    Ok(Registry { catalog, lookup })
}

// What the relay handler leaves for the spend logger: the response text, the billed duration and its cost.
fn deepgram_answer(call: &DeepgramCall) -> Result<Value, Box<HelperError>> {
    let registry = registry(&call.catalog)?;
    let kwargs = PassThroughLoggingKwargs {
        entries: call.kwargs.clone(),
    };
    let request = DeepgramListenRequest {
        upstream_url: &call.upstream_url,
        messages: &call.messages,
        kwargs: &kwargs,
        catalog: &registry.catalog,
        lookup: &registry.lookup,
    };
    let listen = deepgram_listen_outcome(&request).map_err(failure)?;
    let spend = listen.spend;
    Ok(json!({
        "text": transcript_text(&listen.outcome),
        "billed_seconds": listen.billed_seconds.normalize().to_string(),
        "model": spend.model,
        "custom_llm_provider": spend.custom_llm_provider,
        "response_cost": spend.response_cost.map(|cost| cost.normalize().to_string()),
        "kwargs": kwargs_view(&listen.outcome.kwargs, spend.response_cost),
    }))
}

// What the relay handler leaves for the spend logger: the image urls, the model and the submission's cost.
fn fal_answer(call: &FalCall) -> Result<Value, Box<HelperError>> {
    let registry = registry(&call.catalog)?;
    let kwargs = PassThroughLoggingKwargs {
        entries: call.kwargs.clone(),
    };
    let request = FalPassthroughRequest {
        url_route: &call.url_route,
        response_body: &call.response_body,
        request_body: &call.request_body,
        kwargs: &kwargs,
        lookup: &registry.lookup,
        queue_base: call.queue_base.as_deref(),
    };
    let passthrough = fal_ai_passthrough_outcome(&request).map_err(failure)?;
    let spend = passthrough.spend;
    Ok(json!({
        "urls": image_urls(&passthrough.outcome),
        "model": spend.model,
        "custom_llm_provider": spend.custom_llm_provider,
        "response_cost": spend.response_cost.map(|cost| cost.normalize().to_string()),
        "kwargs": kwargs_view(&passthrough.outcome.kwargs, spend.response_cost),
    }))
}

fn transcript_text(outcome: &HandlerOutcome) -> String {
    let Some(PassThroughResponse::Transcription(Json::Object(fields))) = &outcome.result else {
        return String::new();
    };
    let Some(Json::Text(text)) = fields.get("text") else {
        return String::new();
    };
    text.clone()
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

// Money crosses the wire as normalized decimal text; the kwargs carry the same text for their cost.
fn kwargs_view(kwargs: &PassThroughLoggingKwargs, cost: Option<Decimal>) -> Value {
    let mut view = Map::new();
    for (key, value) in &kwargs.entries {
        view.insert(key.clone(), json!(value));
    }
    view.insert(
        "response_cost".to_owned(),
        json!(cost.map(|cost| cost.normalize().to_string())),
    );
    Value::Object(view)
}

fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: std::fmt::Display,
{
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
