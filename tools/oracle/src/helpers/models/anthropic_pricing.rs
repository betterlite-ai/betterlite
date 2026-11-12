use std::collections::BTreeMap;

use betterlite_models::anthropic_pricing::error::AnthropicPricingError;
use betterlite_models::anthropic_pricing::probe::search_requests_from_response;
use betterlite_models::anthropic_pricing::search::web_search_cost;
use betterlite_models::anthropic_pricing::{AnthropicPriceRequest, TokenPriceRequest, token_cost};
use betterlite_models::cost_usage::CostUsage;
use betterlite_models::cost_usage::server_tool_projection::{
    ServerToolProjection, ServerToolRecord, ServerToolUse, web_search_requests,
};
use betterlite_models::decisions::Json;
use betterlite_models::price_fields::nested::SearchContextPricing;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord, PricedModel};
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use chrono::DateTime;
use indexmap::IndexMap;
use jiff::tz::TimeZoneDatabase;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::OrHelperError as _;

pub(crate) struct Op;

struct NullLookup;

impl PriceLookup for NullLookup {
    fn effective_model_info(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PriceRecord> {
        None
    }

    fn raw_cost_map_entry(&self, _key: &str) -> Option<PriceRecord> {
        None
    }

    fn effective_priced_model(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Option<PricedModel> {
        None
    }
}

#[derive(Deserialize)]
pub(crate) struct UsageInput {
    #[serde(default, rename = "prompt_tokens")]
    prompt: i64,
    #[serde(default, rename = "completion_tokens")]
    completion: i64,
    #[serde(default, rename = "total_tokens")]
    total: i64,
}

#[derive(Deserialize)]
pub(crate) struct TokenInput {
    model: String,
    usage: UsageInput,
    #[serde(default)]
    model_info: Option<PriceRecord>,
    #[serde(default)]
    provider_specific_entry: Option<PriceRecord>,
    #[serde(default)]
    service_tier: Option<String>,
    #[serde(default)]
    inference_geo: Option<String>,
    #[serde(default)]
    speed: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RecordInput {
    Mapping { value: IndexMap<String, Json> },
    Attribute { value: ServerToolUse },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Input {
    Token(TokenInput),
    RawProbe {
        responses: Vec<Json>,
    },
    WebSearchRequests {
        #[serde(default)]
        record: Option<RecordInput>,
    },
    SearchCost {
        #[serde(default)]
        model_info: Option<PriceRecord>,
        #[serde(default)]
        search_context: Option<SearchContextPricing>,
        #[serde(default)]
        record: Option<RecordInput>,
    },
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::Token(fields) => run_token(fields),
            Input::RawProbe { responses } => Ok(raw_probe_value(responses)),
            Input::WebSearchRequests { record } => accessor_value(record.as_ref())
                .map_or(Ok(serde_json::Value::Null), |count| {
                    serde_json::to_value(&count).or_value_error()
                }),
            Input::SearchCost {
                model_info,
                search_context,
                record,
            } => run_search_cost(
                model_info.as_ref(),
                search_context.as_ref(),
                record.as_ref(),
            ),
        }
    }
}

const fn usage_record(input: &UsageInput) -> CostUsage {
    CostUsage {
        prompt_tokens: input.prompt,
        completion_tokens: input.completion,
        total_tokens: input.total,
        prompt_details: None,
        completion_details: None,
        cache_read_tokens: None,
        numeric_extras: BTreeMap::new(),
    }
}

fn server_tool_record(input: Option<&RecordInput>) -> Option<ServerToolRecord<'_>> {
    match input? {
        RecordInput::Mapping { value } => Some(ServerToolRecord::Mapping(value)),
        RecordInput::Attribute { value } => Some(ServerToolRecord::Parsed(value)),
    }
}

fn run_token(input: &TokenInput) -> crate::helpers::HelperResult {
    let usage = usage_record(&input.usage);
    let extras = PromptExtras::default();
    let projection = ServerToolProjection {
        server_tool_use: None,
        inference_geo: input.inference_geo.clone(),
        speed: input.speed.clone(),
    };
    let request = TokenPriceRequest::Anthropic(AnthropicPriceRequest {
        model: &input.model,
        usage: &usage,
        prompt_extras: &extras,
        supplied_model_info: input.model_info.as_ref(),
        provider_specific_entry: input.provider_specific_entry.as_ref(),
        service_tier: input.service_tier.as_deref(),
        projection: &projection,
        moment: DateTime::UNIX_EPOCH,
    });
    let result = token_cost(&request, &NullLookup, &TimeZoneDatabase::from_env())
        .map_err(|error| anthropic_error(&error))?;
    Ok(json!({
        "prompt_cost": result.prompt_cost.normalize().to_string(),
        "completion_cost": result.completion_cost.normalize().to_string(),
    }))
}

fn run_search_cost(
    model_info: Option<&PriceRecord>,
    search_context: Option<&SearchContextPricing>,
    record: Option<&RecordInput>,
) -> crate::helpers::HelperResult {
    let cost = web_search_cost(model_info, search_context, server_tool_record(record))
        .map_err(|error| anthropic_error(&error))?;
    Ok(json!({ "cost": cost.normalize().to_string() }))
}

fn raw_probe_value(responses: &[Json]) -> serde_json::Value {
    let values: Vec<serde_json::Value> = responses
        .iter()
        .map(|response| {
            search_requests_from_response(response)
                .map_or(serde_json::Value::Null, |count| json!(count))
        })
        .collect();
    serde_json::Value::Array(values)
}

fn accessor_value(record: Option<&RecordInput>) -> Option<Json> {
    match record? {
        RecordInput::Mapping { value } => {
            web_search_requests(Some(ServerToolRecord::Mapping(value)))
        }
        RecordInput::Attribute { value } => {
            web_search_requests(Some(ServerToolRecord::Parsed(value)))
        }
    }
}

fn anthropic_error(error: &AnthropicPricingError) -> Box<crate::helpers::HelperError> {
    Box::new(crate::helpers::HelperError {
        class: "AnthropicPricingError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

mod tests;
