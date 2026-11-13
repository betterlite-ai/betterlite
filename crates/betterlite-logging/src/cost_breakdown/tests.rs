#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr as _;
use std::time::Duration;

use betterlite_models::billed_tokens::BilledTokenRates;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::cost_breakdown::{CostBreakdownInput, CostBreakdownTarget};
use crate::logging_state::LoggingInit;
use crate::logging_state::LoggingState;
use crate::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use crate::logging_state::types::{CallbackPolicy, DynamicCallbacks, RequestLoggingFlags};

fn amount(text: &str) -> Decimal {
    Decimal::from_str(text).expect("the fixture is a decimal")
}

fn state() -> LoggingState {
    let init = LoggingInit {
        model: "gpt-4o".to_owned(),
        call_type: "completion".to_owned(),
        start_time: DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00")
            .expect("the fixture is RFC 3339"),
        litellm_call_id: "call-1".to_owned(),
        function_id: "fn-1".to_owned(),
        messages: None,
        stream: None,
        litellm_trace_id: None,
        kwargs: IndexMap::new(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    };
    let mut context = CorrelationContext::default();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut context,
            settings: CorrelationSettings::default(),
        },
        &Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor compiles"),
        || "trace-1".to_owned(),
        Duration::default,
    )
    .expect("the fixture builds")
}

fn breakdown_input() -> CostBreakdownInput {
    CostBreakdownInput {
        input_cost: amount("0.001"),
        output_cost: amount("0.002"),
        total_cost: amount("0.003"),
        tool_usage_cost: amount("0.0004"),
        additional_costs: None,
        original_cost: None,
        discount_percent: None,
        discount_amount: None,
        margin_percent: None,
        margin_fixed_amount: None,
        margin_total_amount: None,
        cache_read_cost: None,
        cache_creation_cost: None,
        reasoning_cost: None,
        service_tier: None,
        data_residency: None,
        vertex_location: None,
        billed_token_rates: None,
    }
}

fn rates() -> BilledTokenRates {
    BilledTokenRates {
        input_cost_per_token: amount("0.1"),
        output_cost_per_token: amount("0.2"),
        cache_read_input_token_cost: amount("0.3"),
        audio_read: amount("0.4"),
        cache_write: amount("0.5"),
        cache_write_1h: amount("0.6"),
        reasoning: amount("0.7"),
    }
}

#[test]
fn base_amounts_are_always_stored() {
    let mut state = state();
    state.set_cost_breakdown(&breakdown_input());
    let record = state
        .cost_breakdown
        .as_ref()
        .expect("the breakdown is stored");
    assert_eq!(record.input_cost, Some(amount("0.001")));
    assert_eq!(record.output_cost, Some(amount("0.002")));
    assert_eq!(record.total_cost, Some(amount("0.003")));
    assert_eq!(record.tool_usage_cost, Some(amount("0.0004")));
    assert_eq!(record.guardrail_cost, None);
    assert!(record.extra.is_empty());
}

#[rstest]
#[case::absent(None, None)]
#[case::zero(Some("0"), None)]
#[case::negative(Some("-0.5"), None)]
#[case::positive(Some("0.5"), Some("0.5"))]
fn token_detail_lines_need_a_positive_cost(
    #[case] cache_read: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let mut state = state();
    let mut input = breakdown_input();
    input.cache_read_cost = cache_read.map(amount);
    input.cache_creation_cost = cache_read.map(amount);
    input.reasoning_cost = cache_read.map(amount);
    state.set_cost_breakdown(&input);
    let record = state
        .cost_breakdown
        .as_ref()
        .expect("the breakdown is stored");
    let wanted = expected.map(amount);
    assert_eq!(record.cache_read_cost, wanted);
    assert_eq!(record.cache_creation_cost, wanted);
    assert_eq!(record.reasoning_cost, wanted);
}

#[test]
fn zero_discount_and_margin_fields_are_kept() {
    let mut state = state();
    let mut input = breakdown_input();
    input.original_cost = Some(Decimal::ZERO);
    input.discount_percent = Some(Decimal::ZERO);
    input.discount_amount = Some(Decimal::ZERO);
    input.margin_percent = Some(Decimal::ZERO);
    input.margin_fixed_amount = Some(Decimal::ZERO);
    input.margin_total_amount = Some(Decimal::ZERO);
    state.set_cost_breakdown(&input);
    let record = state
        .cost_breakdown
        .as_ref()
        .expect("the breakdown is stored");
    assert_eq!(record.original_cost, Some(Decimal::ZERO));
    assert_eq!(record.discount_percent, Some(Decimal::ZERO));
    assert_eq!(record.discount_amount, Some(Decimal::ZERO));
    assert_eq!(record.margin_percent, Some(Decimal::ZERO));
    assert_eq!(record.margin_fixed_amount, Some(Decimal::ZERO));
    assert_eq!(record.margin_total_amount, Some(Decimal::ZERO));
}

#[rstest]
#[case::absent(None, None)]
#[case::empty(Some(BTreeMap::new()), None)]
#[case::one(Some(BTreeMap::from([("azure_model_router_flat_cost".to_owned(), amount("0.00014"))])), Some("0.00014"))]
fn additional_costs_are_kept_only_when_nonempty(
    #[case] additional_costs: Option<BTreeMap<String, Decimal>>,
    #[case] expected: Option<&str>,
) {
    let mut state = state();
    let mut input = breakdown_input();
    input.additional_costs = additional_costs;
    state.set_cost_breakdown(&input);
    let record = state
        .cost_breakdown
        .as_ref()
        .expect("the breakdown is stored");
    assert_eq!(
        record
            .additional_costs
            .as_ref()
            .and_then(|costs| costs.get("azure_model_router_flat_cost").copied()),
        expected.map(amount)
    );
    assert_eq!(record.additional_costs.is_none(), expected.is_none());
}

#[test]
fn billing_basis_fields_stay_nullable() {
    let mut state = state();
    let mut input = breakdown_input();
    input.service_tier = Some("default".to_owned());
    input.vertex_location = None;
    state.set_cost_breakdown(&input);
    let record = state
        .cost_breakdown
        .as_ref()
        .expect("the breakdown is stored");
    assert_eq!(record.service_tier.as_deref(), Some("default"));
    assert_eq!(record.data_residency, None);
    assert_eq!(record.vertex_location, None);
}

#[test]
fn a_second_call_replaces_the_whole_record() {
    let mut state = state();
    let mut first = breakdown_input();
    first.additional_costs = Some(BTreeMap::from([(
        "azure_model_router_flat_cost".to_owned(),
        amount("0.00014"),
    )]));
    first.cache_read_cost = Some(amount("0.5"));
    first.data_residency = Some("eu".to_owned());
    first.billed_token_rates = Some(rates());
    state.set_cost_breakdown(&first);

    state.set_cost_breakdown(&breakdown_input());
    let record = state
        .cost_breakdown
        .as_ref()
        .expect("the breakdown is stored");
    assert_eq!(record.additional_costs, None);
    assert_eq!(record.cache_read_cost, None);
    assert_eq!(record.data_residency, None);
    assert_eq!(record.input_cost, Some(amount("0.001")));
    assert_eq!(state.billed_token_rates, None);
}

#[test]
fn billed_rates_follow_the_latest_call() {
    let mut state = state();
    let mut input = breakdown_input();
    input.billed_token_rates = Some(rates());
    state.set_cost_breakdown(&input);
    assert_eq!(state.billed_token_rates, Some(rates()));

    let mut cleared = breakdown_input();
    cleared.billed_token_rates = None;
    state.set_cost_breakdown(&cleared);
    assert_eq!(state.billed_token_rates, None);
}

#[test]
fn the_trait_target_stores_the_same_record() {
    let mut state = state();
    CostBreakdownTarget::set_cost_breakdown(&mut state, &breakdown_input())
        .expect("storing the logging record cannot fail");
    let record = state
        .cost_breakdown
        .as_ref()
        .expect("the breakdown is stored");
    assert_eq!(record.input_cost, Some(amount("0.001")));
}
