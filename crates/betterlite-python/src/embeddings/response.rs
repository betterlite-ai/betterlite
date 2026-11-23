use super::convert::EmbeddingArguments;
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_openai::embedding::{EmbeddingRecord, EmbeddingVector};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Serialize;
use serde_json::Number;
use std::collections::BTreeMap;
// What Python reads back: the fields it builds an EmbeddingResponse from, the hidden params and the timing.
#[derive(Debug, Serialize)]
pub(crate) struct EmbeddingAnswer {
    pub(crate) object: String,
    pub(crate) model: String,
    pub(crate) data: Vec<Json>,
    // a dict, so Python's Usage validates it the way LiteLLM's own Usage(**dict) does
    pub(crate) usage: Json,
    pub(crate) hidden_params: HiddenParams,
    pub(crate) response_headers: BTreeMap<String, String>,
    pub(crate) response_ms: f64,
}
// The values the logging object keeps beside a response; a mock answer carries neither provider nor overhead.
pub(crate) struct HiddenFacts<'facts> {
    pub(crate) call_id: &'facts str,
    pub(crate) api_base: &'facts str,
    pub(crate) litellm_model: &'facts str,
    pub(crate) provider: Option<&'facts str>,
}
#[must_use]
pub(crate) fn hidden_params(
    facts: &HiddenFacts<'_>,
    cost: Json,
    additional_headers: AdditionalHeaders,
    response_ms: f64,
    overhead_ms: Option<f64>,
) -> HiddenParams {
    let mut hidden = HiddenParams::new();
    hidden.insert(
        String::from("_response_ms"),
        Number::from_f64(response_ms).map_or(Json::Null, Json::Number),
    );
    hidden.insert(
        String::from("callback_duration_ms"),
        Number::from_f64(0.0_f64).map_or(Json::Null, Json::Number),
    );
    hidden.insert(
        String::from("litellm_call_id"),
        Json::Text(facts.call_id.to_owned()),
    );
    hidden.insert(
        String::from("api_base"),
        Json::Text(facts.api_base.to_owned()),
    );
    // the OpenAI handler's deployment carries no model_info, so LiteLLM logs a null model id
    hidden.insert(String::from("model_id"), Json::Null);
    hidden.insert(String::from("response_cost"), cost);
    hidden.insert(
        String::from("additional_headers"),
        Json::Object(additional_headers),
    );
    hidden.insert(
        String::from("litellm_model_name"),
        Json::Text(facts.litellm_model.to_owned()),
    );
    if let Some(overhead) = overhead_ms.and_then(Number::from_f64) {
        hidden.insert(
            String::from("litellm_overhead_time_ms"),
            Json::Number(overhead),
        );
    }
    if let Some(provider) = facts.provider {
        hidden.insert(
            String::from("custom_llm_provider"),
            Json::Text(provider.to_owned()),
        );
    }
    hidden
}
// LiteLLM prices a request against the custom pair when both halves are given, and the catalog otherwise
#[must_use]
pub(crate) fn response_cost(
    arguments: &EmbeddingArguments,
    catalog: &ModelCatalog,
    model: &str,
    provider: &str,
    prompt_tokens: u32,
) -> Json {
    let custom = match (
        arguments.input_cost_per_token,
        arguments.output_cost_per_token,
    ) {
        (Some(input), Some(_output)) => Some(input),
        (Some(_input), None) | (None, Some(_output)) | (None, None) => None,
    };
    let price = custom.or_else(|| catalog_price(catalog, model, provider));
    let Some(cost) = price.and_then(|unit| unit.checked_mul(Decimal::from(prompt_tokens))) else {
        return Json::Null;
    };
    cost.to_f64()
        .and_then(Number::from_f64)
        .map_or(Json::Null, Json::Number)
}
fn catalog_price(catalog: &ModelCatalog, model: &str, provider: &str) -> Option<Decimal> {
    let prefixed = format!("{provider}/{model}");
    catalog
        .entries
        .get(model)
        .or_else(|| catalog.entries.get(&prefixed))
        .and_then(|entry| entry.prices.values.input.input_token)
}
// mock_embedding's Usage(prompt_tokens=10, completion_tokens=0) keeps total_tokens at its 0 default
#[must_use]
pub(crate) fn mock_usage() -> Json {
    let mut usage = IndexMap::new();
    usage.insert(
        String::from("prompt_tokens"),
        Json::Number(Number::from(10_u32)),
    );
    usage.insert(
        String::from("completion_tokens"),
        Json::Number(Number::from(0_u32)),
    );
    usage.insert(
        String::from("total_tokens"),
        Json::Number(Number::from(0_u32)),
    );
    Json::Object(usage)
}
pub(crate) fn record_json(record: &EmbeddingRecord) -> Json {
    let mut fields = IndexMap::new();
    fields.insert(String::from("object"), Json::Text(record.object.clone()));
    fields.insert(
        String::from("index"),
        Json::Number(Number::from(record.index)),
    );
    let vector = match &record.embedding {
        EmbeddingVector::Base64(text) => Json::Text(text.clone()),
        EmbeddingVector::Floats(values) => {
            Json::List(values.iter().cloned().map(Json::Number).collect())
        }
    };
    fields.insert(String::from("embedding"), vector);
    Json::Object(fields)
}
