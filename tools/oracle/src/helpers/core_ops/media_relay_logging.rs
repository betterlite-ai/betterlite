use std::fmt::Display;

use betterlite_config::cost_adjustments::CostAdjustmentMap;
use betterlite_core::api::media_relay_logging::routes::{
    is_deepgram_listen_route, is_fal_ai_route,
};
use betterlite_core::api::media_relay_logging::spend::MediaSpend;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use self::deepgram::deepgram_answer;
use self::fal::fal_answer;

mod deepgram;
mod fal;
mod services;

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
    #[serde(default)]
    cost_discount_config: CostAdjustmentMap,
    #[serde(default)]
    cost_margin_config: CostAdjustmentMap,
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

// What the relay handler leaves for the spend logger: the response's own fields, then the model, provider, cost and kwargs.
fn relay_answer(
    mut answer: Map<String, Value>,
    outcome: &HandlerOutcome,
    spend: &MediaSpend,
) -> Value {
    answer.insert("model".to_owned(), json!(spend.model));
    answer.insert(
        "custom_llm_provider".to_owned(),
        json!(spend.custom_llm_provider),
    );
    answer.insert(
        "response_cost".to_owned(),
        json!(spend.response_cost.map(|cost| cost.normalize().to_string())),
    );
    answer.insert(
        "kwargs".to_owned(),
        kwargs_view(&outcome.kwargs, spend.response_cost),
    );
    Value::Object(answer)
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
    Error: Display,
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
