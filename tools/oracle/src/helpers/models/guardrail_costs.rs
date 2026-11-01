use betterlite_models::cost_breakdown::{
    CostBreakdown, CostBreakdownError, cost_breakdown_with_guardrail,
};
use betterlite_models::guardrail_pricing::entries::{
    GuardrailInformation, GuardrailInformationEntry, billed_guardrail_cost_by_unit,
    guardrail_information_cost,
};
use betterlite_models::guardrail_pricing::{azure_prompt_shield_cost, guardrail_cost_total};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(crate) struct Op;

const fn default_guardrail_information() -> GuardrailInformation {
    GuardrailInformation::Null(())
}

#[derive(Deserialize)]
pub(crate) struct AzureScenario {
    #[serde(default)]
    usage_units: BTreeMap<String, i64>,
    #[serde(default)]
    cost_tier: Option<String>,
    #[serde(default)]
    price_per_1000_text_records: Option<Decimal>,
}

#[derive(Deserialize)]
pub(crate) struct BreakdownInput {
    #[serde(default)]
    existing: Option<CostBreakdown>,
    #[serde(default)]
    guardrail_costs: Vec<Decimal>,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    #[serde(default = "default_guardrail_information")]
    guardrail_information: GuardrailInformation,
    #[serde(default)]
    azure: Vec<AzureScenario>,
    #[serde(default)]
    breakdown: Option<BreakdownInput>,
}

fn helper_error(class: &str, message: String) -> Box<crate::helpers::HelperError> {
    let error = crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    };
    Box::new(error)
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let scalar_cost = guardrail_information_cost(&input.guardrail_information)
            .map_err(|error| helper_error("GuardrailPricingError", error.to_string()))?;
        let rollup = rollup_results(&input.guardrail_information)?;
        let azure = azure_results(&input.azure)?;
        let breakdown_steps = breakdown_steps(input.breakdown.as_ref())?;
        Ok(json!({
            "scalar_cost": decimal_text(scalar_cost),
            "rollup": rollup,
            "azure": azure,
            "breakdown_steps": breakdown_steps,
        }))
    }
}

fn entries_of(information: &GuardrailInformation) -> Vec<&GuardrailInformationEntry> {
    match information {
        GuardrailInformation::Null(()) => Vec::new(),
        GuardrailInformation::Array(entries) => entries.iter().collect(),
        GuardrailInformation::Entry(entry) => vec![entry],
    }
}

fn rollup_results(
    information: &GuardrailInformation,
) -> Result<Vec<Value>, Box<crate::helpers::HelperError>> {
    let entries = entries_of(information);
    entries.into_iter().map(rollup_entry_result).collect()
}

fn rollup_entry_result(
    entry: &GuardrailInformationEntry,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let cost_by_unit = billed_guardrail_cost_by_unit(entry)
        .map_err(|error| helper_error("GuardrailPricingError", error.to_string()))?;
    let total = guardrail_cost_total(cost_by_unit.as_ref())
        .map_err(|error| helper_error("GuardrailPricingError", error.to_string()))?;
    Ok(json!({
        "cost_by_unit": cost_by_unit_json(cost_by_unit.as_ref()),
        "total": decimal_text(total),
    }))
}

fn azure_results(
    scenarios: &[AzureScenario],
) -> Result<Vec<Value>, Box<crate::helpers::HelperError>> {
    let mut results = Vec::new();
    for scenario in scenarios {
        results.push(azure_scenario_cost(scenario)?);
    }
    Ok(results)
}

fn azure_scenario_cost(
    scenario: &AzureScenario,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let cost = azure_prompt_shield_cost(
        &scenario.usage_units,
        scenario.cost_tier.as_deref(),
        scenario.price_per_1000_text_records,
    )
    .map_err(|error| helper_error("GuardrailPricingError", error.to_string()))?;
    Ok(cost.map_or(Value::Null, |value| Value::String(decimal_text(value))))
}

fn breakdown_steps(
    breakdown: Option<&BreakdownInput>,
) -> Result<Vec<Value>, Box<crate::helpers::HelperError>> {
    let Some(sequence) = breakdown else {
        return Ok(Vec::new());
    };
    let mut current = sequence.existing.clone();
    let mut steps = Vec::new();
    for guardrail_cost in &sequence.guardrail_costs {
        current = cost_breakdown_with_guardrail(current.as_ref(), *guardrail_cost).map_err(
            |error: CostBreakdownError| helper_error("CostBreakdownError", error.to_string()),
        )?;
        steps.push(current.as_ref().map_or(Value::Null, cost_breakdown_json));
    }
    Ok(steps)
}

fn decimal_text(value: Decimal) -> String {
    let normalized = value.normalize();
    normalized.to_string()
}

fn cost_by_unit_json(cost_by_unit: Option<&BTreeMap<String, Option<Decimal>>>) -> Value {
    let formatted = cost_by_unit.map(|costs| {
        costs
            .iter()
            .map(cost_by_unit_entry)
            .collect::<serde_json::Map<_, _>>()
    });
    formatted.map_or(Value::Null, Value::Object)
}

fn cost_by_unit_entry(entry: (&String, &Option<Decimal>)) -> (String, Value) {
    let (counter, cost) = entry;
    (
        counter.clone(),
        cost.map_or(Value::Null, |value| Value::String(decimal_text(value))),
    )
}

fn cost_breakdown_json(breakdown: &CostBreakdown) -> Value {
    let additional_costs = breakdown.additional_costs.as_ref().map(|costs| {
        costs
            .iter()
            .map(|(key, value)| (key.clone(), decimal_text(*value)))
            .collect::<BTreeMap<String, String>>()
    });
    json!({
        "service_tier": breakdown.service_tier,
        "data_residency": breakdown.data_residency,
        "vertex_location": breakdown.vertex_location,
        "input_cost": breakdown.input_cost.map(decimal_text),
        "cache_read_cost": breakdown.cache_read_cost.map(decimal_text),
        "cache_creation_cost": breakdown.cache_creation_cost.map(decimal_text),
        "output_cost": breakdown.output_cost.map(decimal_text),
        "reasoning_cost": breakdown.reasoning_cost.map(decimal_text),
        "total_cost": breakdown.total_cost.map(decimal_text),
        "tool_usage_cost": breakdown.tool_usage_cost.map(decimal_text),
        "guardrail_cost": breakdown.guardrail_cost.map(decimal_text),
        "additional_costs": additional_costs,
        "original_cost": breakdown.original_cost.map(decimal_text),
        "discount_percent": breakdown.discount_percent.map(decimal_text),
        "discount_amount": breakdown.discount_amount.map(decimal_text),
        "margin_percent": breakdown.margin_percent.map(decimal_text),
        "margin_fixed_amount": breakdown.margin_fixed_amount.map(decimal_text),
        "margin_total_amount": breakdown.margin_total_amount.map(decimal_text),
    })
}
