use std::collections::BTreeMap;

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::guardrail_pricing::{bedrock_guardrail_cost_by_unit, guardrail_cost_total};
use rust_decimal::Decimal;

use super::assessments::violation_category_names;
use super::error::BedrockDecisionError;
use crate::bedrock_records::{Response, Usage};
use crate::guardrail_records::record::GuardrailTracingDetail;

pub struct Pricing<'price> {
    pub catalog: &'price ModelCatalog,
    pub aws_region_name: Option<&'price str>,
}

// LiteLLM's _build_tracing_detail: category names and the action come from the raw response, before redaction
pub fn tracing_detail(
    response: &Response,
    pricing: &Pricing<'_>,
) -> Result<GuardrailTracingDetail, BedrockDecisionError> {
    let categories = violation_category_names(response);
    let usage = usage_tracing_detail(response.usage.as_ref(), pricing)?;
    Ok(detail(
        (!categories.is_empty()).then_some(categories),
        response.action.clone(),
        usage.guardrail_usage,
        usage.guardrail_cost,
        usage.guardrail_cost_by_unit,
    ))
}

// LiteLLM's _usage_tracing_detail: only integer counters are billed, and no counter leaves no detail
pub fn usage_tracing_detail(
    usage: Option<&Usage>,
    pricing: &Pricing<'_>,
) -> Result<GuardrailTracingDetail, BedrockDecisionError> {
    let Some(usage) = usage else {
        return Ok(detail(None, None, None, None, None));
    };
    let mut units = BTreeMap::new();
    for (counter, count) in &usage.0 {
        if let Some(value) = count {
            units.insert(counter.clone(), *value);
        }
    }
    if units.is_empty() {
        return Ok(detail(None, None, None, None, None));
    }
    let cost_by_unit =
        bedrock_guardrail_cost_by_unit(pricing.catalog, pricing.aws_region_name, &units)?;
    let total = guardrail_cost_total(cost_by_unit.as_ref())?;
    Ok(detail(None, None, Some(units), Some(total), cost_by_unit))
}

fn detail(
    violation_categories: Option<Vec<String>>,
    guardrail_action: Option<String>,
    guardrail_usage: Option<BTreeMap<String, i64>>,
    guardrail_cost: Option<Decimal>,
    guardrail_cost_by_unit: Option<BTreeMap<String, Option<Decimal>>>,
) -> GuardrailTracingDetail {
    GuardrailTracingDetail {
        guardrail_id: None,
        policy_template: None,
        detection_method: None,
        confidence_score: None,
        classification: None,
        match_details: None,
        patterns_checked: None,
        alert_recipients: None,
        risk_score: None,
        violation_categories,
        guardrail_action,
        guardrail_usage,
        guardrail_cost,
        guardrail_cost_by_unit,
        guardrail_cost_in_spend: None,
    }
}
