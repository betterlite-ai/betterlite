#![cfg(test)]

use std::cell::RefCell;
use std::collections::BTreeMap;

use betterlite_models::cost_usage::{CompletionDetails, CostUsage, PromptDetails};
use betterlite_models::price_fields::nested::SearchContextPricing;
use betterlite_models::token_cost::GenericCostResult;
use betterlite_models::token_cost::error::TokenCostError;
use rstest::rstest;
use rust_decimal::Decimal;
use serde_json::{Value, json};

use super::error::XaiUsageError;
use super::reconcile::{fold_reasoning, normalize_totals, usage_restated_from_xai_ticks};
use super::{
    GenericTokenPricing, XaiUsage, attach_tool_details, cost_per_token,
    cost_per_web_search_request, web_search_calls, xai_reported_cost_in_usd,
};

struct RecordingPricing {
    seen: RefCell<Vec<(String, String, CostUsage)>>,
    fail: bool,
}

impl GenericTokenPricing for RecordingPricing {
    fn generic_cost_per_token(
        &self,
        model: &str,
        custom_llm_provider: &str,
        usage: &CostUsage,
    ) -> Result<GenericCostResult, TokenCostError> {
        self.seen.borrow_mut().push((
            model.to_owned(),
            custom_llm_provider.to_owned(),
            usage.clone(),
        ));
        if self.fail {
            return Err(TokenCostError::UnknownModel);
        }
        Ok(GenericCostResult {
            prompt_cost: Decimal::new(11, 2),
            completion_cost: Decimal::new(22, 2),
        })
    }
}

fn decimal(text: &str) -> Decimal {
    text.parse().unwrap()
}

fn recording(fail: bool) -> RecordingPricing {
    RecordingPricing {
        seen: RefCell::new(Vec::new()),
        fail,
    }
}

fn usage(prompt: i64, completion: i64, total: i64, reasoning: Option<i64>) -> XaiUsage {
    XaiUsage {
        tokens: CostUsage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: total,
            prompt_details: None,
            completion_details: reasoning.map(|tokens| CompletionDetails {
                text_tokens: None,
                audio_tokens: None,
                image_tokens: None,
                video_tokens: None,
                reasoning_tokens: Some(tokens),
            }),
            cache_read_tokens: None,
            numeric_extras: BTreeMap::new(),
        },
        cost: None,
        cost_in_usd_ticks: None,
        server_side_tool_usage_details: None,
    }
}

fn prompt_details(text: i64, web_search_requests: Option<i64>) -> PromptDetails {
    PromptDetails {
        text_tokens: Some(text),
        audio_tokens: None,
        image_tokens: None,
        video_tokens: None,
        web_search_requests,
    }
}

fn details(calls: Value) -> BTreeMap<String, Value> {
    BTreeMap::from([("web_search_calls".to_owned(), calls)])
}

fn pricing(medium: Option<&str>, low: Option<&str>, high: Option<&str>) -> SearchContextPricing {
    SearchContextPricing {
        search_context_size_medium: medium.map(decimal),
        search_context_size_low: low.map(decimal),
        search_context_size_high: high.map(decimal),
    }
}

#[rstest]
#[case::whole_dollar(json!(10_000_000_000_u64), Some("1"))]
#[case::one_tick(json!(1_i32), Some("0.0000000001"))]
#[case::zero(json!(0_i32), Some("0"))]
#[case::negative_zero(serde_json::from_str("-0").unwrap(), Some("0"))]
#[case::beyond_double_precision(json!(123_456_789_012_345_678_u64), Some("12345678.9012345678"))]
#[case::negative(json!(-1_i32), None)]
#[case::boolean_true(json!(true), None)]
#[case::boolean_false(json!(false), None)]
#[case::float(json!(5.0_f64), None)]
#[case::exponent(serde_json::from_str("1e3").unwrap(), None)]
#[case::text(json!("100"), None)]
#[case::null(Value::Null, None)]
#[case::array(json!([1_i32]), None)]
#[case::object(serde_json::from_str(r#"{"ticks":1}"#).unwrap(), None)]
#[case::beyond_decimal_range(serde_json::from_str("100000000000000000000000000000000000000000").unwrap(), None)]
fn xai_usage_ticks_divide_exactly_and_reject_non_integers(
    #[case] ticks: Value,
    #[case] expected: Option<&str>,
) {
    assert_eq!(xai_reported_cost_in_usd(&ticks), expected.map(decimal));
}

#[rstest]
#[case::reported_zero(Some("0"), "0", "0")]
#[case::reported_positive(Some("0.25"), "0", "0.25")]
fn xai_usage_reported_cost_bypasses_token_prices(
    #[case] reported: Option<&str>,
    #[case] prompt: &str,
    #[case] completion: &str,
) {
    let mut priced = usage(100, 50, 150, Some(10));
    priced.cost = reported.map(decimal);
    let delegate = recording(false);
    let result = cost_per_token(&delegate, "grok-4", &priced).unwrap();
    assert_eq!(result, (decimal(prompt), decimal(completion)));
    assert!(delegate.seen.borrow().is_empty());
}

#[test]
fn xai_usage_negative_reported_cost_falls_through_to_token_prices() {
    let mut priced = usage(100, 50, 150, None);
    priced.cost = Some(decimal("-0.01"));
    let delegate = recording(false);
    let result = cost_per_token(&delegate, "grok-4", &priced).unwrap();
    assert_eq!(result, (decimal("0.11"), decimal("0.22")));
    assert_eq!(delegate.seen.borrow().len(), 1);
}

#[rstest]
#[case::folded_total_matches(100, 60, 160, Some(10), 60)]
#[case::raw_total_includes_reasoning(100, 50, 160, Some(10), 60)]
#[case::raw_total_without_reasoning_details(100, 50, 160, None, 50)]
#[case::total_equals_visible_tokens(100, 50, 150, Some(10), 50)]
#[case::zero_reasoning_changes_nothing(100, 50, 160, Some(0), 50)]
fn xai_usage_pricing_counts_reasoning_once(
    #[case] prompt: i64,
    #[case] completion: i64,
    #[case] total: i64,
    #[case] reasoning: Option<i64>,
    #[case] priced_completion: i64,
) {
    let delegate = recording(false);
    cost_per_token(
        &delegate,
        "grok-4",
        &usage(prompt, completion, total, reasoning),
    )
    .unwrap();
    let seen = delegate.seen.borrow();
    let (model, provider, adjusted) = &seen[0];
    assert_eq!((model.as_str(), provider.as_str()), ("grok-4", "xai"));
    assert_eq!(adjusted.completion_tokens, priced_completion);
    assert_eq!(adjusted.total_tokens, total);
    assert_eq!(adjusted.completion_details, None);
}

#[test]
fn xai_usage_pricing_passes_prompt_details_through() {
    let mut priced = usage(100, 50, 150, None);
    priced.tokens.prompt_details = Some(prompt_details(80, None));
    priced.tokens.cache_read_tokens = Some(20);
    let delegate = recording(false);
    cost_per_token(&delegate, "grok-4", &priced).unwrap();
    let seen = delegate.seen.borrow();
    assert_eq!(seen[0].2.prompt_details, priced.tokens.prompt_details);
    assert_eq!(seen[0].2.cache_read_tokens, Some(20));
}

#[test]
fn xai_usage_pricing_surfaces_delegate_errors_and_overflow() {
    let delegate = recording(true);
    let error = cost_per_token(&delegate, "grok-4", &usage(1, 1, 2, None)).unwrap_err();
    assert!(matches!(
        error,
        XaiUsageError::Pricing(TokenCostError::UnknownModel)
    ));
    let overflow = cost_per_token(&delegate, "grok-4", &usage(i64::MAX, 1, 0, None)).unwrap_err();
    assert!(matches!(overflow, XaiUsageError::Overflow));
}

#[rstest]
#[case::raw_shape_is_folded(100, 50, 160, Some(10), 60)]
#[case::folded_shape_stays(100, 60, 160, Some(10), 60)]
#[case::visible_total_stays(100, 50, 150, Some(10), 50)]
#[case::total_is_neither(100, 50, 175, Some(10), 50)]
#[case::no_reasoning(100, 50, 160, None, 50)]
#[case::negative_reasoning(100, 50, 140, Some(-10), 50)]
fn xai_usage_fold_adds_reasoning_only_when_the_total_includes_it(
    #[case] prompt: i64,
    #[case] completion: i64,
    #[case] total: i64,
    #[case] reasoning: Option<i64>,
    #[case] folded_completion: i64,
) {
    let mut folding = usage(prompt, completion, total, reasoning);
    fold_reasoning(&mut folding).unwrap();
    assert_eq!(folding.tokens.completion_tokens, folded_completion);
    assert_eq!(folding.tokens.total_tokens, total);
}

#[rstest]
#[case::raw_shape(usage(100, 50, 160, Some(10)))]
#[case::folded_shape(usage(100, 60, 160, Some(10)))]
#[case::deficient_total(usage(100, 60, 120, Some(10)))]
fn xai_usage_reconciliation_is_idempotent(#[case] mut once: XaiUsage) {
    fold_reasoning(&mut once).unwrap();
    normalize_totals(&mut once).unwrap();
    let mut twice = once.clone();
    fold_reasoning(&mut twice).unwrap();
    normalize_totals(&mut twice).unwrap();
    assert_eq!(once, twice);
}

#[rstest]
#[case::deficient_total_is_raised(100, 50, 120, 150)]
#[case::exact_total_stays(100, 50, 150, 150)]
#[case::larger_total_is_kept(100, 50, 400, 400)]
#[case::zero_total_is_raised(100, 50, 0, 150)]
fn xai_usage_normalize_raises_a_deficient_total_without_lowering_a_larger_one(
    #[case] prompt: i64,
    #[case] completion: i64,
    #[case] total: i64,
    #[case] expected: i64,
) {
    let mut normalizing = usage(prompt, completion, total, None);
    normalize_totals(&mut normalizing).unwrap();
    assert_eq!(normalizing.tokens.total_tokens, expected);
}

#[test]
fn xai_usage_reconciliation_reports_overflow() {
    let mut overflowing = usage(i64::MAX, 1, 0, Some(1));
    assert!(matches!(
        fold_reasoning(&mut overflowing),
        Err(XaiUsageError::Overflow)
    ));
    assert!(matches!(
        normalize_totals(&mut overflowing),
        Err(XaiUsageError::Overflow)
    ));
}

#[test]
fn xai_usage_restated_cost_replaces_only_the_cost() {
    let mut original = usage(100, 50, 150, Some(10));
    original.cost = Some(decimal("9"));
    original.cost_in_usd_ticks = Some(json!(25_000_000_i32));
    original.server_side_tool_usage_details = Some(details(json!(2_i32)));
    let restated = usage_restated_from_xai_ticks(&original).unwrap();
    assert_eq!(restated.cost, Some(decimal("0.0025")));
    assert_eq!(
        XaiUsage {
            cost: original.cost,
            ..restated
        },
        original
    );
}

#[rstest]
#[case::no_ticks(None)]
#[case::boolean_ticks(Some(json!(true)))]
#[case::negative_ticks(Some(json!(-5_i32)))]
#[case::float_ticks(Some(json!(2.5_f64)))]
fn xai_usage_restated_cost_needs_valid_ticks(#[case] ticks: Option<Value>) {
    let mut original = usage(1, 1, 2, None);
    original.cost_in_usd_ticks = ticks;
    assert_eq!(usage_restated_from_xai_ticks(&original), None);
}

#[test]
fn xai_usage_restated_cost_accepts_zero_ticks() {
    let mut original = usage(1, 1, 2, None);
    original.cost_in_usd_ticks = Some(json!(0_i32));
    original.cost = Some(decimal("4"));
    let restated = usage_restated_from_xai_ticks(&original).unwrap();
    assert_eq!(restated.cost, Some(Decimal::ZERO));
}

#[rstest]
#[case::integer(json!(3_i32), Some(3))]
#[case::zero(json!(0_i32), Some(0))]
#[case::null(Value::Null, Some(0))]
#[case::float_truncates(json!(2.9_f64), Some(2))]
#[case::numeric_text(json!("4"), Some(4))]
#[case::padded_text(json!(" 4 "), Some(4))]
#[case::empty_text(json!(""), Some(0))]
#[case::decimal_text(json!("2.5"), None)]
#[case::word(json!("many"), None)]
#[case::underscored_text(json!("1_0"), None)]
#[case::arabic_indic_digits(json!("\u{663}"), None)]
#[case::boolean_true(json!(true), Some(1))]
#[case::boolean_false(json!(false), Some(0))]
#[case::array(json!([1_i32]), None)]
#[case::object(serde_json::from_str(r#"{"n":1}"#).unwrap(), None)]
#[case::beyond_i64(serde_json::from_str("100000000000000000000").unwrap(), None)]
fn xai_usage_search_count_converts_like_an_integer(
    #[case] calls: Value,
    #[case] expected: Option<i64>,
) {
    assert_eq!(web_search_calls(&details(calls)), expected);
}

#[test]
fn xai_usage_search_count_defaults_to_zero_when_missing() {
    assert_eq!(web_search_calls(&BTreeMap::new()), Some(0));
}

#[rstest]
#[case::positive(json!(2_i32), Some(2))]
#[case::zero(json!(0_i32), None)]
#[case::negative(json!(-1_i32), None)]
#[case::malformed(json!("many"), None)]
fn xai_usage_search_details_attach_even_when_the_count_is_malformed(
    #[case] calls: Value,
    #[case] requests: Option<i64>,
) {
    let mut attaching = usage(1, 1, 2, None);
    let attached = details(calls);
    attach_tool_details(&mut attaching, Some(attached.clone()));
    assert_eq!(attaching.server_side_tool_usage_details, Some(attached));
    let counted = attaching
        .tokens
        .prompt_details
        .and_then(|prompt| prompt.web_search_requests);
    assert_eq!(counted, requests);
}

#[test]
fn xai_usage_search_details_keep_existing_prompt_details() {
    let mut attaching = usage(1, 1, 2, None);
    attaching.tokens.prompt_details = Some(prompt_details(7, None));
    attach_tool_details(&mut attaching, Some(details(json!(3_i32))));
    let prompt = attaching.tokens.prompt_details.unwrap();
    assert_eq!(
        (prompt.text_tokens, prompt.web_search_requests),
        (Some(7), Some(3))
    );
}

#[test]
fn xai_usage_search_details_absent_leave_usage_alone() {
    let mut attaching = usage(1, 1, 2, None);
    attach_tool_details(&mut attaching, None);
    assert_eq!(attaching, usage(1, 1, 2, None));
}

#[rstest]
#[case::medium_first(pricing(Some("0.01"), Some("0.02"), Some("0.03")), "0.03")]
#[case::low_when_medium_is_zero(pricing(Some("0"), Some("0.02"), Some("0.03")), "0.06")]
#[case::low_when_medium_is_missing(pricing(None, Some("0.02"), Some("0.03")), "0.06")]
#[case::high_when_others_are_not_positive(pricing(Some("0"), Some("0"), Some("0.04")), "0.12")]
#[case::default_when_nothing_is_positive(pricing(Some("0"), None, Some("0")), "0.015")]
#[case::default_for_empty_pricing(SearchContextPricing::default(), "0.015")]
fn xai_usage_search_rate_tries_medium_low_high_then_the_default(
    #[case] rates: SearchContextPricing,
    #[case] expected: &str,
) {
    let mut searching = usage(1, 1, 2, None);
    searching.server_side_tool_usage_details = Some(details(json!(3_i32)));
    let cost = cost_per_web_search_request(&searching, Some(&rates)).unwrap();
    assert_eq!(cost, decimal(expected));
}

#[test]
fn xai_usage_search_rate_defaults_without_model_pricing() {
    let mut searching = usage(1, 1, 2, None);
    searching.server_side_tool_usage_details = Some(details(json!(2_i32)));
    let cost = cost_per_web_search_request(&searching, None).unwrap();
    assert_eq!(cost, decimal("0.01"));
}

#[rstest]
#[case::no_details(None, None)]
#[case::zero_calls(Some(json!(0_i32)), None)]
#[case::negative_calls(Some(json!(-2_i32)), None)]
#[case::malformed_calls(Some(json!("many")), None)]
#[case::reported_zero(Some(json!(4_i32)), Some("0"))]
#[case::reported_positive(Some(json!(4_i32)), Some("1.5"))]
fn xai_usage_search_charge_is_zero_when_nothing_is_billable(
    #[case] calls: Option<Value>,
    #[case] reported: Option<&str>,
) {
    let mut searching = usage(1, 1, 2, None);
    searching.server_side_tool_usage_details = calls.map(details);
    searching.cost = reported.map(decimal);
    let cost = cost_per_web_search_request(&searching, Some(&pricing(Some("0.01"), None, None)));
    assert_eq!(cost.unwrap(), Decimal::ZERO);
}

#[test]
fn xai_usage_search_charge_reports_overflow() {
    let mut searching = usage(1, 1, 2, None);
    searching.server_side_tool_usage_details = Some(details(json!(i64::MAX)));
    let rates = pricing(Some("79228162514264337593543950335"), None, None);
    assert!(matches!(
        cost_per_web_search_request(&searching, Some(&rates)),
        Err(XaiUsageError::Overflow)
    ));
}
