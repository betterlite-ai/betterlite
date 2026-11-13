use std::collections::BTreeMap;
use std::fmt::Display;

use betterlite_models::decisions::Json;
use betterlite_models::price_lookup::PriceRecord;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;

mod execute;
mod model;
mod projection;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    function: Function,
    #[serde(default)]
    catalog: Vec<CatalogEntry>,
    #[serde(default)]
    identity: Option<IdentityInput>,
    #[serde(default)]
    response_time: Option<ResponseTimeInput>,
    #[serde(default)]
    breakdowns: Vec<BreakdownInput>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    Identity,
    ResponseTime,
    Breakdown,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CatalogEntry {
    key: String,
    entry: PriceRecord,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IdentityInput {
    model: Option<String>,
    #[serde(default)]
    response: Option<ResponseInput>,
    #[serde(default)]
    base_model: Option<String>,
    #[serde(default)]
    custom_pricing: Option<bool>,
    custom_llm_provider: String,
    #[serde(default)]
    router_model_id: Option<String>,
    #[serde(default)]
    region_name: Option<String>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResponseKind {
    Object,
    Dictionary,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseInput {
    kind: ResponseKind,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    hidden_params: Option<IndexMap<String, Json>>,
    #[serde(default)]
    response_ms: Option<Json>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResponseTimeInput {
    response: ResponseInput,
    #[serde(default)]
    total_time: Option<Decimal>,
    #[serde(default)]
    timing: Option<TimingInput>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TimingInput {
    start_time: String,
    end_time: String,
    now: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BreakdownInput {
    input_cost: Decimal,
    output_cost: Decimal,
    total_cost: Decimal,
    tool_usage_cost: Decimal,
    #[serde(default)]
    additional_costs: Option<BTreeMap<String, Decimal>>,
    #[serde(default)]
    original_cost: Option<Decimal>,
    #[serde(default)]
    discount_percent: Option<Decimal>,
    #[serde(default)]
    discount_amount: Option<Decimal>,
    #[serde(default)]
    margin_percent: Option<Decimal>,
    #[serde(default)]
    margin_fixed_amount: Option<Decimal>,
    #[serde(default)]
    margin_total_amount: Option<Decimal>,
    #[serde(default)]
    cache_read_cost: Option<Decimal>,
    #[serde(default)]
    cache_creation_cost: Option<Decimal>,
    #[serde(default)]
    reasoning_cost: Option<Decimal>,
    #[serde(default)]
    service_tier: Option<String>,
    #[serde(default)]
    data_residency: Option<String>,
    #[serde(default)]
    vertex_location: Option<String>,
    #[serde(default)]
    billed_token_rates: Option<RatesInput>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RatesInput {
    input_cost_per_token: Decimal,
    output_cost_per_token: Decimal,
    cache_read_input_token_cost: Decimal,
    audio_read: Decimal,
    cache_write: Decimal,
    cache_write_1h: Decimal,
    reasoning: Decimal,
}

type Failure = Box<crate::helpers::HelperError>;

fn failure<Error>(error: Error) -> Failure
where
    Error: Display,
{
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        Ok(serde_json::Value::Array(
            input.calls.iter().map(execute::run_call).collect(),
        ))
    }
}
