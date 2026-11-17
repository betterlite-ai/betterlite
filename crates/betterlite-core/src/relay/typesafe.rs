use std::sync::Arc;

use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_success::payload_state::STANDARD_LOGGING_OBJECT;
use betterlite_logging::logging_success::ports::StandardPayloadBuilder;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::{
    PassThroughResponse, PassThroughResponseObject,
};
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Number;

use crate::api::media_relay_logging::spend::decimal_json;

mod envelope;
mod tests;

const OPENROUTER: &str = "openrouter";
const UNKNOWN_MODEL: &str = "unknown";

// LiteLLM's _TypeSafeResponse and _TypeSafeUsage: absent fields take their defaults and extra keys are ignored.
#[derive(Debug, Default, Deserialize)]
struct TypesafeResponse {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<TypesafeUsage>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct TypesafeUsage {
    #[serde(default)]
    input_tokens: i64,
    #[serde(default)]
    output_tokens: i64,
    #[serde(default)]
    cost: Option<Decimal>,
}

pub struct TypesafePassthroughInput<'call> {
    pub logging_obj: &'call StreamLoggingObject,
    pub response_body: &'call Json,
    pub request_body: &'call Json,
    pub custom_llm_provider: &'call str,
    pub url_route: &'call str,
    pub result: &'call str,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
    pub cache_hit: bool,
    pub kwargs: PassThroughLoggingKwargs,
}

pub struct TypesafePassthroughLogger {
    payload_builder: Arc<dyn StandardPayloadBuilder>,
    catalog: Arc<ModelCatalog>,
}

impl TypesafePassthroughLogger {
    #[must_use]
    pub fn new(
        payload_builder: Arc<dyn StandardPayloadBuilder>,
        catalog: Arc<ModelCatalog>,
    ) -> Self {
        Self {
            payload_builder,
            catalog,
        }
    }

    // LiteLLM has no try/except here: a failed parse or lookup reads as zero tokens and zero cost and never fails the log.
    #[must_use]
    pub fn typesafe_passthrough_handler(
        &self,
        input: &TypesafePassthroughInput<'_>,
    ) -> HandlerOutcome {
        let response = parse_response(input.response_body);
        let provider = input.custom_llm_provider;
        let requested_model = body_model(input.request_body);
        let logged_model = [response.model.as_deref(), Some(requested_model)]
            .into_iter()
            .flatten()
            .find(|model| !model.is_empty())
            .unwrap_or(UNKNOWN_MODEL);
        let model_name = format!("{provider}/{logged_model}");
        let usage = response.usage.clone().unwrap_or_default();
        let response_cost = self.response_cost(input, &response, requested_model, &usage);
        let usage_json = usage_object(&usage);
        let mut kwargs = record(input, &model_name, &usage_json, response_cost);
        let init_response =
            envelope::response_envelope(&model_name, usage_json, input.end_time.timestamp());
        let payload = self.build_payload(input, &init_response);
        kwargs.entries.insert(
            STANDARD_LOGGING_OBJECT.to_owned(),
            payload_entry(payload.as_ref()),
        );
        HandlerOutcome {
            result: Some(standard_result(input.result)),
            kwargs,
        }
    }

    // usage.cost wins only for openrouter; every other call prices its token counts from the catalog.
    fn response_cost(
        &self,
        input: &TypesafePassthroughInput<'_>,
        response: &TypesafeResponse,
        requested_model: &str,
        usage: &TypesafeUsage,
    ) -> Decimal {
        if input.custom_llm_provider == OPENROUTER
            && let Some(cost) = usage.cost
        {
            return cost;
        }
        let keys = candidate_keys(
            input.custom_llm_provider,
            response.model.as_deref(),
            requested_model,
        );
        let (input_rate, output_rate) = token_rates(&self.catalog, &keys);
        token_cost(
            usage.input_tokens,
            usage.output_tokens,
            input_rate,
            output_rate,
        )
    }

    fn build_payload(
        &self,
        input: &TypesafePassthroughInput<'_>,
        init_response: &Json,
    ) -> Option<StandardLoggingPayload> {
        input
            .logging_obj
            .read(&mut |state| {
                self.payload_builder
                    .build(state, init_response, input.start_time, input.end_time)
            })
            .flatten()
    }
}

fn standard_result(result: &str) -> PassThroughResponse {
    PassThroughResponse::Standard(PassThroughResponseObject {
        response: Json::Text(result.to_owned()),
    })
}

// LiteLLM validates the whole record in one go, so one failing field reads every other field as its default.
fn parse_response(body: &Json) -> TypesafeResponse {
    serde_json::to_string(body)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn body_model(body: &Json) -> &str {
    let Json::Object(fields) = body else {
        return "";
    };
    let Some(Json::Text(model)) = fields.get("model") else {
        return "";
    };
    model.as_str()
}

// litellm.model_cost holds alias keys beside the entries they name, so the lookup resolves one like ModelCatalog::metadata does.
fn token_rates(catalog: &ModelCatalog, keys: &[String]) -> (Decimal, Decimal) {
    for key in keys {
        let Some(canonical) = catalog.lookup.get(key.as_str()) else {
            continue;
        };
        let Some(entry) = catalog.entries.get(canonical.as_ref()) else {
            continue;
        };
        let prices = &entry.prices.values;
        return (
            prices.input.input_token.unwrap_or(Decimal::ZERO),
            prices.output.output_token.unwrap_or(Decimal::ZERO),
        );
    }
    (Decimal::ZERO, Decimal::ZERO)
}

fn token_cost(
    prompt_tokens: i64,
    completion_tokens: i64,
    input_rate: Decimal,
    output_rate: Decimal,
) -> Decimal {
    let prompt_cost = input_rate.saturating_mul(Decimal::from(prompt_tokens));
    let completion_cost = output_rate.saturating_mul(Decimal::from(completion_tokens));
    prompt_cost.saturating_add(completion_cost)
}

// Both models are candidate keys even when one is empty, since Python checks for None alone.
fn candidate_keys(
    provider: &str,
    response_model: Option<&str>,
    request_model: &str,
) -> Vec<String> {
    [response_model, Some(request_model)]
        .into_iter()
        .flatten()
        .map(|model| format!("{provider}/{model}"))
        .collect()
}

// Usage(prompt_tokens=..., completion_tokens=..., total_tokens=...) dumps the token counts, then the two absent detail objects.
fn usage_object(usage: &TypesafeUsage) -> Json {
    let total_tokens = usage.input_tokens.saturating_add(usage.output_tokens);
    Json::Object(IndexMap::from([
        (
            "completion_tokens".to_owned(),
            Json::Number(Number::from(usage.output_tokens)),
        ),
        (
            "prompt_tokens".to_owned(),
            Json::Number(Number::from(usage.input_tokens)),
        ),
        (
            "total_tokens".to_owned(),
            Json::Number(Number::from(total_tokens)),
        ),
        ("completion_tokens_details".to_owned(), Json::Null),
        ("prompt_tokens_details".to_owned(), Json::Null),
    ]))
}

// LiteLLM returns the model, provider, cost and combined usage in kwargs and reads the usage from there; the call details carry it here for the builder.
fn record(
    input: &TypesafePassthroughInput<'_>,
    model_name: &str,
    usage_json: &Json,
    response_cost: Decimal,
) -> PassThroughLoggingKwargs {
    let details = details(input, model_name, usage_json, response_cost);
    let mut kwargs = input.kwargs.clone();
    kwargs.entries.extend(details.clone());
    input.logging_obj.edit(&mut |state| {
        model_name.clone_into(&mut state.model);
        state.custom_llm_provider = Some(input.custom_llm_provider.to_owned());
        for (key, value) in &details {
            state.model_call_details.insert(key.clone(), value.clone());
        }
    });
    kwargs
}

fn details(
    input: &TypesafePassthroughInput<'_>,
    model_name: &str,
    usage_json: &Json,
    response_cost: Decimal,
) -> IndexMap<String, Json> {
    IndexMap::from([
        ("model".to_owned(), Json::Text(model_name.to_owned())),
        (
            "custom_llm_provider".to_owned(),
            Json::Text(input.custom_llm_provider.to_owned()),
        ),
        ("response_cost".to_owned(), decimal_json(response_cost)),
        ("combined_usage_object".to_owned(), usage_json.clone()),
    ])
}

// A payload that cannot be written as JSON stands in as Null, the way a failed build does in LiteLLM.
fn payload_entry(payload: Option<&StandardLoggingPayload>) -> Json {
    payload
        .and_then(|built| serde_json::to_string(built).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Json::Null)
}
