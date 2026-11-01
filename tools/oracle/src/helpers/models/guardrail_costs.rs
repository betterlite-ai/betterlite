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

#[derive(Debug, thiserror::Error)]
enum RunError {
    #[error(transparent)]
    GuardrailPricing(#[from] betterlite_models::guardrail_pricing::GuardrailPricingError),
    #[error(transparent)]
    CostBreakdown(#[from] CostBreakdownError),
}

impl From<RunError> for Box<crate::helpers::HelperError> {
    fn from(error: RunError) -> Self {
        let class = match &error {
            RunError::GuardrailPricing(_) => "GuardrailPricingError",
            RunError::CostBreakdown(_) => "CostBreakdownError",
        };
        Self::new(crate::helpers::HelperError {
            class: class.to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    }
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

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let scalar_cost: Decimal =
            guardrail_information_cost(&input.guardrail_information).map_err(RunError::from)?;
        let rollup = entries_of(&input.guardrail_information)
            .into_iter()
            .map(rollup_entry_result)
            .collect::<Result<Vec<Value>, _>>()?;
        let azure = azure_results(&input.azure)?;
        let breakdown_steps = breakdown_steps(input.breakdown.as_ref())?;
        Ok(json!({
            "scalar_cost": scalar_cost.normalize().to_string(),
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

fn rollup_entry_result(
    entry: &GuardrailInformationEntry,
) -> Result<Value, Box<crate::helpers::HelperError>> {
    let cost_by_unit = billed_guardrail_cost_by_unit(entry).map_err(RunError::from)?;
    let total = guardrail_cost_total(cost_by_unit.as_ref()).map_err(RunError::from)?;
    Ok(json!({
        "cost_by_unit": cost_by_unit
            .as_ref()
            .map(|costs| costs.iter().map(cost_by_unit_entry).collect::<serde_json::Map<_, _>>())
            .map_or(Value::Null, Value::Object),
        "total": total.normalize().to_string(),
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
    .map_err(RunError::from)?;
    Ok(cost.map_or(Value::Null, |value| {
        Value::String(value.normalize().to_string())
    }))
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
        current = cost_breakdown_with_guardrail(current.as_ref(), *guardrail_cost)
            .map_err(RunError::from)?;
        steps.push(current.as_ref().map_or(Value::Null, cost_breakdown_json));
    }
    Ok(steps)
}

fn cost_by_unit_entry(entry: (&String, &Option<Decimal>)) -> (String, Value) {
    let (counter, cost) = entry;
    (
        counter.clone(),
        cost.map_or(Value::Null, |value| {
            Value::String(value.normalize().to_string())
        }),
    )
}

const DECIMAL_BREAKDOWN_FIELDS: [&str; 14] = [
    "input_cost",
    "cache_read_cost",
    "cache_creation_cost",
    "output_cost",
    "reasoning_cost",
    "total_cost",
    "tool_usage_cost",
    "guardrail_cost",
    "original_cost",
    "discount_percent",
    "discount_amount",
    "margin_percent",
    "margin_fixed_amount",
    "margin_total_amount",
];

fn known_breakdown_fields(breakdown: &CostBreakdown) -> serde_json::Map<String, Value> {
    let decimal_fields = [
        breakdown.input_cost,
        breakdown.cache_read_cost,
        breakdown.cache_creation_cost,
        breakdown.output_cost,
        breakdown.reasoning_cost,
        breakdown.total_cost,
        breakdown.tool_usage_cost,
        breakdown.guardrail_cost,
        breakdown.original_cost,
        breakdown.discount_percent,
        breakdown.discount_amount,
        breakdown.margin_percent,
        breakdown.margin_fixed_amount,
        breakdown.margin_total_amount,
    ];
    let mut fields = serde_json::Map::new();
    fields.insert("service_tier".to_owned(), json!(breakdown.service_tier));
    fields.insert("data_residency".to_owned(), json!(breakdown.data_residency));
    fields.insert(
        "vertex_location".to_owned(),
        json!(breakdown.vertex_location),
    );
    for (name, value) in DECIMAL_BREAKDOWN_FIELDS.into_iter().zip(decimal_fields) {
        fields.insert(
            name.to_owned(),
            value.map_or(Value::Null, |decimal| {
                Value::String(decimal.normalize().to_string())
            }),
        );
    }
    fields
}

fn cost_breakdown_json(breakdown: &CostBreakdown) -> Value {
    let mut fields = known_breakdown_fields(breakdown);
    let additional_costs = breakdown.additional_costs.as_ref().map(|costs| {
        costs
            .iter()
            .map(|(key, value)| (key.clone(), value.normalize().to_string()))
            .collect::<BTreeMap<String, String>>()
    });
    fields.insert("additional_costs".to_owned(), json!(additional_costs));
    let extra = breakdown
        .extra
        .iter()
        .map(|(key, value)| (key.clone(), value.normalize().to_string()))
        .collect::<BTreeMap<String, String>>();
    fields.insert("extra".to_owned(), json!(extra));
    Value::Object(fields)
}
