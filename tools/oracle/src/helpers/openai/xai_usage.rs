use std::collections::BTreeMap;

use betterlite_models::cost_usage::{CompletionDetails, CostUsage};
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_models::price_fields::nested::SearchContextPricing;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::token_cost::error::TokenCostError;
use betterlite_openai::xai_usage::error::XaiUsageError;
use betterlite_openai::xai_usage::reconcile::{
    fold_reasoning, normalize_totals, usage_restated_from_xai_ticks,
};
use betterlite_openai::xai_usage::{
    GenericTokenPricing, XaiUsage, attach_tool_details, cost_per_token,
    cost_per_web_search_request, xai_reported_cost_in_usd,
};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Number, Value, json};

use crate::helpers::HelperError;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    action: Action,
    #[serde(default)]
    usage: UsageFields,
    #[serde(default)]
    ticks: Value,
    details: Option<BTreeMap<String, Value>>,
    search_costs: Option<SearchContextPricing>,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Action {
    Ticks,
    Cost,
    Reconcile,
    Search,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct UsageFields {
    prompt_tokens: i64,
    completion_tokens: i64,
    total_tokens: i64,
    reasoning_tokens: Option<i64>,
    cost: Option<Number>,
}

// the delegate bills the counts it receives, so the result shows which tokens the price covered
struct CountPricing;

impl GenericTokenPricing for CountPricing {
    fn generic_cost_per_token(
        &self,
        _model: &str,
        _custom_llm_provider: &str,
        usage: &CostUsage,
    ) -> Result<GenericCostResult, TokenCostError> {
        Ok(GenericCostResult {
            prompt_cost: Decimal::from(usage.prompt_tokens),
            completion_cost: Decimal::from(usage.completion_tokens),
        })
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input.action {
            Action::Ticks => Ok(ticks(&input.ticks)),
            Action::Cost => priced(&input.usage),
            Action::Reconcile => reconciled(&input.usage),
            Action::Search => searched(
                &input.usage,
                input.details.as_ref(),
                input.search_costs.as_ref(),
            ),
        }
    }
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn usage(fields: &UsageFields) -> Result<XaiUsage, Box<HelperError>> {
    let cost = fields
        .cost
        .as_ref()
        .map(|number| parse_decimal_lexeme(&number.to_string()))
        .transpose()
        .map_err(|error| failure(error.to_string()))?;
    Ok(XaiUsage {
        tokens: CostUsage {
            prompt_tokens: fields.prompt_tokens,
            completion_tokens: fields.completion_tokens,
            total_tokens: fields.total_tokens,
            prompt_details: None,
            completion_details: fields.reasoning_tokens.map(|reasoning| CompletionDetails {
                text_tokens: None,
                audio_tokens: None,
                image_tokens: None,
                video_tokens: None,
                reasoning_tokens: Some(reasoning),
            }),
            cache_read_tokens: None,
            numeric_extras: BTreeMap::new(),
        },
        cost,
        cost_in_usd_ticks: None,
        server_side_tool_usage_details: None,
    })
}

fn counts(usage: &XaiUsage) -> Value {
    json!({
        "prompt_tokens": usage.tokens.prompt_tokens,
        "completion_tokens": usage.tokens.completion_tokens,
        "total_tokens": usage.tokens.total_tokens,
    })
}

fn ticks(ticks: &Value) -> Value {
    let carrier = XaiUsage {
        tokens: CostUsage::default(),
        cost: None,
        cost_in_usd_ticks: Some(ticks.clone()),
        server_side_tool_usage_details: None,
    };
    let restated = usage_restated_from_xai_ticks(&carrier).and_then(|restated| restated.cost);
    json!({
        "cost": xai_reported_cost_in_usd(ticks).map(|amount| amount.normalize().to_string()),
        "restated_cost": restated.map(|amount| amount.normalize().to_string()),
    })
}

fn priced(fields: &UsageFields) -> crate::helpers::HelperResult {
    let (prompt_cost, completion_cost) =
        cost_per_token(&CountPricing, "oracle-model", &usage(fields)?)
            .map_err(|error: XaiUsageError| failure(error.to_string()))?;
    let prompt = prompt_cost.normalize().to_string();
    let completion = completion_cost.normalize().to_string();
    Ok(json!({ "prompt_cost": prompt, "completion_cost": completion }))
}

fn reconciled(fields: &UsageFields) -> crate::helpers::HelperResult {
    let mut usage = usage(fields)?;
    fold_reasoning(&mut usage).map_err(|error| failure(error.to_string()))?;
    normalize_totals(&mut usage).map_err(|error| failure(error.to_string()))?;
    let once = counts(&usage);
    fold_reasoning(&mut usage).map_err(|error| failure(error.to_string()))?;
    normalize_totals(&mut usage).map_err(|error| failure(error.to_string()))?;
    Ok(json!({ "once": once, "twice": counts(&usage) }))
}

fn searched(
    fields: &UsageFields,
    details: Option<&BTreeMap<String, Value>>,
    search_costs: Option<&SearchContextPricing>,
) -> crate::helpers::HelperResult {
    let mut usage = usage(fields)?;
    attach_tool_details(&mut usage, details.cloned());
    let search_cost = cost_per_web_search_request(&usage, search_costs)
        .map_err(|error| failure(error.to_string()))?;
    let requests = usage
        .tokens
        .prompt_details
        .and_then(|prompt| prompt.web_search_requests);
    Ok(json!({
        "attached": usage.server_side_tool_usage_details,
        "web_search_requests": requests,
        "search_cost": search_cost.normalize().to_string(),
    }))
}

mod tests;
