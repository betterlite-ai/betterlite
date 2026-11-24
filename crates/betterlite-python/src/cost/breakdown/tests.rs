#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_logging::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget as _};
use betterlite_models::billed_tokens::BilledTokenRates;
use rust_decimal::Decimal;

use crate::cost::breakdown::Capture;

fn dec(text: &str) -> Decimal {
    Decimal::from_str(text).expect("the fixture decimal parses")
}

fn input() -> CostBreakdownInput {
    CostBreakdownInput {
        input_cost: dec("0.01"),
        output_cost: dec("0.02"),
        total_cost: dec("0.03"),
        tool_usage_cost: dec("0.04"),
        additional_costs: None,
        original_cost: Some(dec("0.1")),
        discount_percent: Some(dec("0.05")),
        discount_amount: Some(dec("0.005")),
        margin_percent: Some(dec("0.1")),
        margin_fixed_amount: Some(dec("0.001")),
        margin_total_amount: Some(dec("0.002")),
        cache_read_cost: None,
        cache_creation_cost: None,
        reasoning_cost: None,
        service_tier: Some("flex".to_owned()),
        data_residency: Some("eu".to_owned()),
        vertex_location: Some("global".to_owned()),
        billed_token_rates: Some(BilledTokenRates {
            input_cost_per_token: dec("0.001"),
            output_cost_per_token: dec("0.002"),
            cache_read_input_token_cost: dec("0.0001"),
            audio_read: dec("0.0002"),
            cache_write: dec("0.0003"),
            cache_write_1h: dec("0.0004"),
            reasoning: dec("0.0005"),
        }),
    }
}

#[test]
fn cost_breakdown_records_every_charged_field() {
    let mut capture = Capture::default();
    capture
        .set_cost_breakdown(&input())
        .expect("the capture cannot fail");
    let breakdown = capture.breakdown.expect("the breakdown is recorded");
    assert_eq!(breakdown.input_cost, Some(dec("0.01")));
    assert_eq!(breakdown.output_cost, Some(dec("0.02")));
    assert_eq!(breakdown.total_cost, Some(dec("0.03")));
    assert_eq!(breakdown.tool_usage_cost, Some(dec("0.04")));
    assert!(breakdown.guardrail_cost.is_none());
    assert!(breakdown.extra.is_empty());
    assert!(capture.rates.is_some());
}

#[test]
fn cost_breakdown_records_the_adjustments_and_the_region() {
    let mut capture = Capture::default();
    capture
        .set_cost_breakdown(&input())
        .expect("the capture cannot fail");
    let breakdown = capture.breakdown.expect("the breakdown is recorded");
    assert_eq!(breakdown.original_cost, Some(dec("0.1")));
    assert_eq!(breakdown.discount_percent, Some(dec("0.05")));
    assert_eq!(breakdown.discount_amount, Some(dec("0.005")));
    assert_eq!(breakdown.margin_percent, Some(dec("0.1")));
    assert_eq!(breakdown.margin_fixed_amount, Some(dec("0.001")));
    assert_eq!(breakdown.margin_total_amount, Some(dec("0.002")));
    assert_eq!(breakdown.service_tier.as_deref(), Some("flex"));
    assert_eq!(breakdown.data_residency.as_deref(), Some("eu"));
    assert_eq!(breakdown.vertex_location.as_deref(), Some("global"));
}

#[test]
fn cost_breakdown_keeps_a_positive_cache_and_reasoning_cost() {
    let mut charged = input();
    charged.cache_read_cost = Some(dec("0.001"));
    charged.cache_creation_cost = Some(dec("0.002"));
    charged.reasoning_cost = Some(dec("0.003"));
    let mut capture = Capture::default();
    capture
        .set_cost_breakdown(&charged)
        .expect("the capture cannot fail");
    let breakdown = capture.breakdown.expect("the breakdown is recorded");
    assert_eq!(breakdown.cache_read_cost, Some(dec("0.001")));
    assert_eq!(breakdown.cache_creation_cost, Some(dec("0.002")));
    assert_eq!(breakdown.reasoning_cost, Some(dec("0.003")));
}

#[test]
fn cost_breakdown_keeps_a_zero_cache_cost_and_drops_an_empty_map() {
    let mut charged = input();
    charged.cache_read_cost = Some(Decimal::ZERO);
    charged.cache_creation_cost = Some(Decimal::ZERO);
    charged.reasoning_cost = Some(Decimal::ZERO);
    charged.additional_costs = Some(BTreeMap::new());
    charged.billed_token_rates = None;
    let mut capture = Capture::default();
    capture
        .set_cost_breakdown(&charged)
        .expect("the capture cannot fail");
    let breakdown = capture.breakdown.expect("the breakdown is recorded");
    assert_eq!(breakdown.cache_read_cost, Some(Decimal::ZERO));
    assert_eq!(breakdown.cache_creation_cost, Some(Decimal::ZERO));
    assert_eq!(breakdown.reasoning_cost, Some(Decimal::ZERO));
    assert!(breakdown.additional_costs.is_none());
    assert!(capture.rates.is_none());
}

#[test]
fn cost_breakdown_keeps_a_filled_additional_map() {
    let mut charged = input();
    let mut costs = BTreeMap::new();
    costs.insert("azure_model_router_flat_cost".to_owned(), dec("0.00014"));
    charged.additional_costs = Some(costs);
    let mut capture = Capture::default();
    capture
        .set_cost_breakdown(&charged)
        .expect("the capture cannot fail");
    let breakdown = capture.breakdown.expect("the breakdown is recorded");
    let found = breakdown.additional_costs.expect("the map is kept");
    assert_eq!(
        found.get("azure_model_router_flat_cost"),
        Some(&dec("0.00014"))
    );
}
