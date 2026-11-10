use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_core::cost::context::pricing::{deployment_info, ocr_info};
use betterlite_core::cost::context::response_time::{ResponseTiming, response_time_ms};
use betterlite_core::cost::context::{
    CostResponse, IdentityRequest, ProviderInference, billing_model_name,
};
use betterlite_logging::cost_breakdown::CostBreakdownTarget;
use betterlite_logging::fields::timestamps::Moment;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::billed_tokens::BilledTokenRates;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::resolution::ProviderIdentity;
use betterlite_models::tiered_pricing::Scalar;
use rust_decimal::Decimal;
use serde_json::{Value, json};

use super::model::{CaseLookup, breakdown_input, cost_response, logging_state, moment};
use super::{Call, Failure, Function, IdentityInput, TimingInput, failure};

struct NoInference;

impl ProviderInference for NoInference {
    type Failure = &'static str;

    fn inferred_provider(&self, _model: &str) -> Result<ProviderIdentity, Self::Failure> {
        Err("the case supplies the provider")
    }
}

pub(super) fn run_call(call: &Call) -> Value {
    match execute(call) {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(error) => json!({"ok": false, "error": {"class": error.class}}),
    }
}

fn execute(call: &Call) -> Result<Value, Failure> {
    match call.function {
        Function::Identity => identity(call),
        Function::ResponseTime => response_time(call),
        Function::Deployment => deployment(call),
        Function::Ocr => ocr(call),
        Function::Breakdown => breakdown(call),
    }
}

fn identity(call: &Call) -> Result<Value, Failure> {
    let input = required(call.identity.as_ref(), "identity")?;
    let lookup = CaseLookup::from(call.catalog.as_slice());
    let response = input.response.as_ref().map(cost_response);
    let request = request(input, response.as_ref());
    let selected = billing_model_name(&request, &lookup, &NoInference);
    Ok(selected.map_or(Value::Null, Value::from))
}

fn request<'request>(
    input: &'request IdentityInput,
    response: Option<&'request CostResponse<'request>>,
) -> IdentityRequest<'request> {
    IdentityRequest {
        model: input.model.as_deref(),
        response,
        base_model: input.base_model.as_deref(),
        custom_pricing: input.custom_pricing,
        custom_llm_provider: Some(&input.custom_llm_provider),
        router_model_id: input.router_model_id.as_deref(),
        region_name: input.region_name.as_deref(),
    }
}

fn response_time(call: &Call) -> Result<Value, Failure> {
    let input = required(call.response_time.as_ref(), "response_time")?;
    let response = cost_response(&input.response);
    let moments = moment_values(input.timing.as_ref())?;
    let timing = moments.as_ref().map(|(start, end, now)| ResponseTiming {
        start_time: Some(start),
        end_time: Some(end),
        now,
    });
    let observed =
        response_time_ms(Some(&response), input.total_time, timing.as_ref()).map_err(failure)?;
    Ok(Value::from(observed.normalize().to_string()))
}

fn moment_values(
    timing: Option<&TimingInput>,
) -> Result<Option<(Moment, Moment, Moment)>, Failure> {
    let Some(found) = timing else {
        return Ok(None);
    };
    Ok(Some((
        moment(&found.start_time)?,
        moment(&found.end_time)?,
        moment(&found.now)?,
    )))
}

fn deployment(call: &Call) -> Result<Value, Failure> {
    let input = required(call.pricing.as_ref(), "pricing")?;
    let lookup = CaseLookup::from(call.catalog.as_slice());
    let record = deployment_info(
        input.custom_pricing,
        input.router_model_id.as_deref(),
        input.litellm_params.as_ref(),
        &lookup,
    )
    .map_err(failure)?;
    Ok(deployment_value(record.as_ref()))
}

fn deployment_value(record: Option<&PriceRecord>) -> Value {
    record.map_or(Value::Null, |entry| {
        let fields: BTreeMap<String, Value> = entry
            .iter()
            .map(|(key, scalar)| (key.clone(), scalar_value(scalar)))
            .collect();
        json!({"fields": fields})
    })
}

fn ocr(call: &Call) -> Result<Value, Failure> {
    let input = required(call.pricing.as_ref(), "pricing")?;
    let lookup = CaseLookup::from(call.catalog.as_slice());
    let pricing = ocr_info(
        input.custom_pricing,
        input.router_model_id.as_deref(),
        input.litellm_params.as_ref(),
        &lookup,
    )
    .map_err(failure)?;
    Ok(pricing.map_or(Value::Null, |layered| {
        json!({
            "ocr_cost_per_page": optional_decimal(layered.ocr_cost_per_page),
            "ocr_cost_per_credit": optional_decimal(layered.ocr_cost_per_credit),
            "annotation_cost_per_page": optional_decimal(layered.annotation_cost_per_page),
        })
    }))
}

fn breakdown(call: &Call) -> Result<Value, Failure> {
    let mut state = logging_state()?;
    let mut records = Vec::with_capacity(call.breakdowns.len());
    for input in &call.breakdowns {
        let converted = breakdown_input(input);
        CostBreakdownTarget::set_cost_breakdown(&mut state, &converted).map_err(failure)?;
        records.push(record_value(&state));
    }
    Ok(Value::Array(records))
}

fn record_value(state: &LoggingState) -> Value {
    json!({
        "cost_breakdown": state.cost_breakdown.as_ref().map_or(Value::Null, |record| json!({
            "input_cost": optional_decimal(record.input_cost),
            "output_cost": optional_decimal(record.output_cost),
            "total_cost": optional_decimal(record.total_cost),
            "tool_usage_cost": optional_decimal(record.tool_usage_cost),
            "cache_read_cost": optional_decimal(record.cache_read_cost),
            "cache_creation_cost": optional_decimal(record.cache_creation_cost),
            "reasoning_cost": optional_decimal(record.reasoning_cost),
            "additional_costs": optional_costs(record.additional_costs.as_ref()),
            "original_cost": optional_decimal(record.original_cost),
            "discount_percent": optional_decimal(record.discount_percent),
            "discount_amount": optional_decimal(record.discount_amount),
            "margin_percent": optional_decimal(record.margin_percent),
            "margin_fixed_amount": optional_decimal(record.margin_fixed_amount),
            "margin_total_amount": optional_decimal(record.margin_total_amount),
            "service_tier": optional_text(record.service_tier.as_deref()),
            "data_residency": optional_text(record.data_residency.as_deref()),
            "vertex_location": optional_text(record.vertex_location.as_deref()),
        })),
        "billed_token_rates": state.billed_token_rates.map_or(Value::Null, rates_value),
    })
}

fn rates_value(rates: BilledTokenRates) -> Value {
    json!({
        "input_cost_per_token": rates.input_cost_per_token.normalize().to_string(),
        "output_cost_per_token": rates.output_cost_per_token.normalize().to_string(),
        "cache_read_input_token_cost": rates.cache_read_input_token_cost.normalize().to_string(),
        "audio_read": rates.audio_read.normalize().to_string(),
        "cache_write": rates.cache_write.normalize().to_string(),
        "cache_write_1h": rates.cache_write_1h.normalize().to_string(),
        "reasoning": rates.reasoning.normalize().to_string(),
    })
}

fn optional_decimal(value: Option<Decimal>) -> Value {
    let Some(amount) = value else {
        return Value::Null;
    };
    Value::from(amount.normalize().to_string())
}

fn optional_text(value: Option<&str>) -> Value {
    let Some(text) = value else {
        return Value::Null;
    };
    Value::from(text)
}

fn optional_costs(costs: Option<&BTreeMap<String, Decimal>>) -> Value {
    costs.map_or(Value::Null, |amounts| {
        let rendered: BTreeMap<String, String> = amounts
            .iter()
            .map(|(key, amount)| (key.clone(), amount.normalize().to_string()))
            .collect();
        json!(rendered)
    })
}

fn scalar_value(scalar: &Scalar) -> Value {
    match scalar {
        Scalar::Null => Value::Null,
        Scalar::Bool(flag) => Value::Bool(*flag),
        Scalar::Number(number) => match Decimal::from_str(&number.to_string()) {
            Ok(value) => Value::from(value.normalize().to_string()),
            Err(_error) => Value::from(number.to_string()),
        },
        Scalar::Text(text) => Value::from(text.clone()),
        Scalar::Array(_) => Value::from("array"),
        Scalar::Object(_) => Value::from("object"),
    }
}

fn required<'value, Item>(
    value: Option<&'value Item>,
    field: &str,
) -> Result<&'value Item, Failure> {
    let Some(found) = value else {
        return Err(failure(format!("{field} is required")));
    };
    Ok(found)
}
