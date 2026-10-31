#![cfg(test)]

use super::*;
use serde_json::json;

fn model(value: Value) -> ModelInfo {
    value
        .as_object()
        .map(|object| {
            object
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn service_alias_and_invalid_rate() {
    let prices = model(
        json!({"input_cost_per_token": "0.1", "input_cost_per_token_priority": "0.2", "output_cost_per_token_priority": "bad"}),
    );
    let rates =
        get_token_base_cost(&prices, Usage { prompt_tokens: 1 }, Some("FAST"), None).unwrap();
    assert_eq!(rates.input_rate, Decimal::new(2, 1));
    assert_eq!(rates.output_rate, Decimal::ZERO);
}

#[test]
fn batch_families_are_independent() {
    let prices = model(
        json!({"input_cost_per_token_batches": "1", "input_cost_per_token_above_1k_tokens_batches": "2", "output_cost_per_token_batches": "3"}),
    );
    let rates = get_batch_cost_rates(
        &prices,
        Usage {
            prompt_tokens: 2_000,
        },
        None,
    )
    .unwrap();
    assert_eq!(rates.input, Some(Decimal::from(2u8)));
    assert_eq!(rates.output, Some(Decimal::from(3u8)));
}

#[test]
fn component_only_multiplies_positive_usage() {
    let prices = model(json!({"input_cost_per_token": "0.2"}));
    assert_eq!(
        calculate_cost_component(&prices, "input_cost_per_token", Some(Decimal::from(-1))),
        Ok(Decimal::ZERO)
    );
    assert_eq!(
        calculate_cost_component(&prices, "input_cost_per_token", Some(Decimal::from(3))),
        Ok(Decimal::new(6, 1))
    );
}

#[test]
fn deployment_keeps_explicit_zero() {
    let prices = model(json!({"input_cost_per_token": 0, "unrelated": 4}));
    let selected = deployment_pricing(Some(&prices)).unwrap().unwrap();
    assert_eq!(
        selected.get("input_cost_per_token"),
        Some(&Value::String("0".to_owned()))
    );
    assert_eq!(selected.len(), 1);
}
