use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;
use std::str::FromStr;

use super::billing::{UsageSource, usage_as_model};

pub enum CostTarget<'a> {
    Model(&'a ResponsesApiResponse),
    Object(&'a IndexMap<String, Json>),
}

pub trait ResponsesCostCalculator {
    type Error: std::error::Error + Send + Sync + 'static;

    fn response_cost(&self, target: &CostTarget<'_>) -> Result<Option<Decimal>, Self::Error>;
}

pub fn stamp_usage_cost<C: ResponsesCostCalculator>(
    response: &mut ResponsesApiResponse,
    calculator: Option<&C>,
) {
    let Some(cost_calculator) = calculator else {
        return;
    };
    if response
        .usage
        .as_ref()
        .is_none_or(|usage| usage.cost.is_some())
    {
        return;
    }
    let answer = cost_calculator.response_cost(&CostTarget::Model(response));
    let Ok(Some(cost)) = answer else {
        return;
    };
    if cost <= Decimal::ZERO {
        return;
    }
    if let Some(usage) = response.usage.as_mut() {
        usage.cost = Some(normalized_cost(cost));
    }
}

pub fn stamp_object_usage_cost<C: ResponsesCostCalculator>(
    object: &mut IndexMap<String, Json>,
    calculator: Option<&C>,
) {
    let Some(cost_calculator) = calculator else {
        return;
    };
    let Some(usage_value) = object.get("usage") else {
        return;
    };
    let Some(usage) = usage_as_model(Some(UsageSource::Value(usage_value))) else {
        return;
    };
    let Ok(usage_json) =
        serde_json::to_value(&usage).and_then(|value| serde_json::from_value::<Json>(value))
    else {
        return;
    };
    object.insert("usage".to_owned(), usage_json);
    if usage.cost.is_some() {
        return;
    }
    let answer = cost_calculator.response_cost(&CostTarget::Object(object));
    let Ok(Some(cost)) = answer else {
        return;
    };
    if cost <= Decimal::ZERO {
        return;
    }
    if let Some(Json::Object(fields)) = object.get_mut("usage") {
        fields.insert("cost".to_owned(), cost_value(cost));
    }
}

fn normalized_cost(cost: Decimal) -> Decimal {
    let mut normalized = cost.normalize();
    if normalized.scale() == 0 {
        normalized.rescale(1);
    }
    normalized
}

fn cost_value(cost: Decimal) -> Json {
    match Number::from_str(&normalized_cost(cost).to_string()) {
        Ok(number) => Json::Number(number),
        Err(_error) => Json::Null,
    }
}
