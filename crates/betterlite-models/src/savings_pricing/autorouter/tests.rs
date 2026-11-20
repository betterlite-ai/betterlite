#![cfg(test)]

use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;

use crate::price_lookup::PricedModel;
use crate::savings_evidence::snapshot::CostBreakdown;
use crate::savings_evidence::usage::{LoggedUsage, PromptTokensDetails};
use crate::savings_pricing::tests::{TestContext, decimal, priced, record};
use crate::savings_pricing::usage::RequestUsage;
use crate::tiered_pricing::Scalar;

use super::logging::{LoggingMetadata, LoggingPayloadInput, savings_for_logging_payload};
use super::{
    AutoRouterDecision, AutoRouterProvenance, AutoRouterRequestInput, AutoRouterSavingsInput,
    autorouter_savings_for_request, compute_autorouter_savings,
};

fn moment() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("the fixture timestamp is valid")
}

fn zone_db() -> TimeZoneDatabase {
    TimeZoneDatabase::from_env()
}

fn request_usage(prompt_tokens: i64) -> RequestUsage {
    RequestUsage {
        usage: LoggedUsage {
            prompt_tokens,
            completion_tokens: 0,
            total_tokens: prompt_tokens,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        },
        cache_creation_token_details: None,
    }
}

fn cache_usage(cached: i64, written: i64) -> RequestUsage {
    RequestUsage {
        usage: LoggedUsage {
            prompt_tokens: 10,
            completion_tokens: 0,
            total_tokens: 10,
            cache_read_input_tokens: cached,
            cache_creation_input_tokens: written,
            prompt_tokens_details: Some(PromptTokensDetails {
                cached_tokens: cached,
                cache_creation_tokens: written,
                cache_write_tokens: written,
                text_tokens: 10 - cached - written,
                audio_tokens: 0,
                image_tokens: 0,
                video_tokens: 0,
                cache_creation_token_details: None,
            }),
            completion_tokens_details: None,
        },
        cache_creation_token_details: None,
    }
}

fn base_input<'a>(
    usage: &'a RequestUsage,
    database: &'a TimeZoneDatabase,
) -> AutoRouterSavingsInput<'a> {
    let issued_at = moment();
    AutoRouterSavingsInput {
        baseline_model: Some("claude"),
        selected_model: Some("other"),
        selected_provider: None,
        usage,
        selected_info: None,
        baseline_info: None,
        cost_breakdown: None,
        baseline_deployment_id: None,
        selected_deployment_id: None,
        baseline_usage: None,
        baseline_provenance: None,
        moment: issued_at,
        database,
    }
}

fn identity_context() -> TestContext {
    TestContext::new()
        .with_identity("claude", None, ("claude", "anthropic"))
        .with_identity("anthropic/claude", None, ("claude", "anthropic"))
        .with_identity("other", None, ("other", "openai"))
}

fn parse_breakdown(json: &str) -> CostBreakdown {
    serde_json::from_str(json).expect("the fixture is a cost breakdown")
}

fn parse_decision(json: &str) -> AutoRouterDecision {
    serde_json::from_str(json).expect("the fixture is a routing decision")
}

#[test]
fn compute_returns_none_when_an_identity_is_missing() {
    let usage = request_usage(10);
    let database = zone_db();
    let input = base_input(&usage, &database);
    assert_eq!(
        compute_autorouter_savings(&TestContext::new(), &input),
        None
    );
    let baseline_only = TestContext::new().with_identity("claude", None, ("claude", "anthropic"));
    assert_eq!(compute_autorouter_savings(&baseline_only, &input), None);
}

#[test]
fn compute_returns_the_signed_difference() {
    let usage = request_usage(10);
    let database = zone_db();
    let baseline_info = priced(&record(r#"{"input_cost_per_token": 2.0}"#));
    let selected_info = priced(&record(r#"{"input_cost_per_token": 1.0}"#));
    let mut input = base_input(&usage, &database);
    input.baseline_info = Some(&baseline_info);
    input.selected_info = Some(&selected_info);
    let savings =
        compute_autorouter_savings(&identity_context(), &input).expect("both sides price");
    assert_eq!(savings, decimal("10"));
}

#[test]
fn compute_keeps_a_loss_negative() {
    let usage = request_usage(10);
    let database = zone_db();
    let baseline_info = priced(&record(r#"{"input_cost_per_token": 1.0}"#));
    let selected_info = priced(&record(r#"{"input_cost_per_token": 2.0}"#));
    let mut input = base_input(&usage, &database);
    input.baseline_info = Some(&baseline_info);
    input.selected_info = Some(&selected_info);
    let savings =
        compute_autorouter_savings(&identity_context(), &input).expect("both sides price");
    assert_eq!(savings, decimal("-10"));
}

#[test]
fn compute_uses_the_recorded_selected_cost() {
    let usage = request_usage(10);
    let database = zone_db();
    let breakdown = parse_breakdown(r#"{"input_cost": 3, "output_cost": 2}"#);
    let baseline_info = priced(&record(r#"{"input_cost_per_token": 2.0}"#));
    let selected_info = priced(&record(r#"{"input_cost_per_token": 1.0}"#));
    let mut input = base_input(&usage, &database);
    input.baseline_info = Some(&baseline_info);
    input.selected_info = Some(&selected_info);
    input.cost_breakdown = Some(&breakdown);
    let savings =
        compute_autorouter_savings(&identity_context(), &input).expect("both sides price");
    assert_eq!(savings, decimal("15"));
}

#[test]
fn compute_normalizes_unpriced_baseline_cache() {
    let usage = cache_usage(2, 3);
    let database = zone_db();
    let info = priced(&record(
        r#"{"input_cost_per_token": 1.0, "cache_read_input_token_cost": 0.5}"#,
    ));
    let mut input = base_input(&usage, &database);
    input.baseline_info = Some(&info);
    input.selected_info = Some(&info);
    input.baseline_usage = Some(&usage);
    let savings =
        compute_autorouter_savings(&identity_context(), &input).expect("both sides price");
    assert_eq!(savings, decimal("3"));
}

#[test]
fn compute_keeps_an_empty_supplied_baseline_info() {
    let usage = request_usage(10);
    let database = zone_db();
    let empty = priced(&record("{}"));
    let selected_info = priced(&record(r#"{"input_cost_per_token": 1.0}"#));
    let context = identity_context().with_public(
        "claude",
        "anthropic",
        record(r#"{"input_cost_per_token": 2.0}"#),
    );
    let mut input = base_input(&usage, &database);
    input.baseline_info = Some(&empty);
    input.selected_info = Some(&selected_info);
    let savings = compute_autorouter_savings(&context, &input).expect("the empty table prices");
    assert_eq!(savings, decimal("-10"));
}

#[test]
fn compute_requires_baseline_usage_for_cache_buckets() {
    let usage = cache_usage(4, 0);
    let database = zone_db();
    let info = priced(&record(
        r#"{"input_cost_per_token": 1.0, "cache_read_input_token_cost": 0.5,
            "cache_creation_input_token_cost": 1.0}"#,
    ));
    let mut input = base_input(&usage, &database);
    input.baseline_info = Some(&info);
    input.selected_info = Some(&info);
    assert_eq!(
        compute_autorouter_savings(&identity_context(), &input),
        None
    );
    input.baseline_usage = Some(&usage);
    assert_eq!(
        compute_autorouter_savings(&identity_context(), &input),
        Some(decimal("0"))
    );
    let writes = cache_usage(0, 4);
    let mut write_input = base_input(&writes, &database);
    write_input.baseline_info = Some(&info);
    write_input.selected_info = Some(&info);
    assert_eq!(
        compute_autorouter_savings(&identity_context(), &write_input),
        None
    );
}

fn observed_input<'a>(
    usage: &'a RequestUsage,
    database: &'a TimeZoneDatabase,
    prices: &'a PricedModel,
    breakdown: &'a CostBreakdown,
) -> AutoRouterSavingsInput<'a> {
    let mut input = base_input(usage, database);
    input.selected_model = Some("anthropic/claude");
    input.baseline_info = Some(prices);
    input.selected_info = Some(prices);
    input.cost_breakdown = Some(breakdown);
    input.baseline_deployment_id = Some("d1");
    input.selected_deployment_id = Some("d1");
    input.baseline_usage = Some(usage);
    input.baseline_provenance = Some(AutoRouterProvenance::ObservedInitial);
    input
}

#[test]
fn observed_initial_is_zero_when_every_evidence_agrees() {
    let usage = request_usage(10);
    let database = zone_db();
    let prices = priced(&record(r#"{"input_cost_per_token": 2.0}"#));
    let breakdown = parse_breakdown(r#"{"input_cost": 10, "output_cost": 10}"#);
    let input = observed_input(&usage, &database, &prices, &breakdown);
    assert_eq!(
        compute_autorouter_savings(&identity_context(), &input),
        Some(decimal("0"))
    );
}

#[test]
fn observed_initial_is_none_without_a_deployment_id() {
    let usage = request_usage(10);
    let database = zone_db();
    let prices = priced(&record(r#"{"input_cost_per_token": 2.0}"#));
    let breakdown = parse_breakdown(r#"{"input_cost": 10, "output_cost": 10}"#);
    let mut input = observed_input(&usage, &database, &prices, &breakdown);
    input.baseline_deployment_id = None;
    assert_eq!(
        compute_autorouter_savings(&identity_context(), &input),
        None
    );
    input.baseline_deployment_id = Some("");
    input.selected_deployment_id = Some("");
    assert_eq!(
        compute_autorouter_savings(&identity_context(), &input),
        None
    );
}

#[test]
fn observed_initial_is_none_when_the_ids_differ() {
    let usage = request_usage(10);
    let database = zone_db();
    let prices = priced(&record(r#"{"input_cost_per_token": 2.0}"#));
    let breakdown = parse_breakdown(r#"{"input_cost": 10, "output_cost": 10}"#);
    let mut input = observed_input(&usage, &database, &prices, &breakdown);
    input.selected_deployment_id = Some("d2");
    assert_eq!(
        compute_autorouter_savings(&identity_context(), &input),
        None
    );
}

#[test]
fn observed_initial_is_none_when_the_prices_differ() {
    let usage = request_usage(10);
    let database = zone_db();
    let prices = priced(&record(r#"{"input_cost_per_token": 2.0}"#));
    let other = priced(&record(r#"{"input_cost_per_token": 3.0}"#));
    let breakdown = parse_breakdown(r#"{"input_cost": 10, "output_cost": 10}"#);
    let mut input = observed_input(&usage, &database, &prices, &breakdown);
    input.selected_info = Some(&other);
    assert_eq!(
        compute_autorouter_savings(&identity_context(), &input),
        None
    );
}

#[test]
fn observed_initial_is_none_when_the_usage_differs() {
    let usage = request_usage(10);
    let database = zone_db();
    let prices = priced(&record(r#"{"input_cost_per_token": 2.0}"#));
    let breakdown = parse_breakdown(r#"{"input_cost": 10, "output_cost": 10}"#);
    let mut other_usage = request_usage(10);
    other_usage.usage.total_tokens = 99;
    let mut input = observed_input(&usage, &database, &prices, &breakdown);
    input.baseline_usage = Some(&other_usage);
    assert_eq!(
        compute_autorouter_savings(&identity_context(), &input),
        None
    );
}

#[test]
fn observed_initial_is_none_without_the_matching_recorded_cost() {
    let usage = request_usage(10);
    let database = zone_db();
    let prices = priced(&record(r#"{"input_cost_per_token": 2.0}"#));
    let missing = parse_breakdown(r#"{"input_cost": 10}"#);
    let input = observed_input(&usage, &database, &prices, &missing);
    assert_eq!(
        compute_autorouter_savings(&identity_context(), &input),
        None
    );
    let different = parse_breakdown(r#"{"input_cost": 1, "output_cost": 1}"#);
    let input = observed_input(&usage, &database, &prices, &different);
    assert_eq!(
        compute_autorouter_savings(&identity_context(), &input),
        None
    );
}

#[test]
fn modeled_provenance_returns_the_difference() {
    let usage = request_usage(10);
    let database = zone_db();
    let prices = priced(&record(r#"{"input_cost_per_token": 2.0}"#));
    let breakdown = parse_breakdown(r#"{"input_cost": 1, "output_cost": 1}"#);
    let mut input = observed_input(&usage, &database, &prices, &breakdown);
    input.baseline_provenance = Some(AutoRouterProvenance::Modeled);
    assert_eq!(
        compute_autorouter_savings(&identity_context(), &input),
        Some(decimal("18"))
    );
}

fn request_context() -> TestContext {
    TestContext::new()
        .with_identity("selected", None, ("selected", "openai"))
        .with_identity("baseline", None, ("baseline", "anthropic"))
        .with_deployment("d-base", record(r#"{"input_cost_per_token": 2.0}"#))
        .with_deployment("d-sel", record(r#"{"input_cost_per_token": 1.0}"#))
        .with_public(
            "baseline",
            "anthropic",
            record(r#"{"input_cost_per_token": 3.0}"#),
        )
}

fn request_input<'a>(
    usage: &'a RequestUsage,
    decision: &'a AutoRouterDecision,
    database: &'a TimeZoneDatabase,
) -> AutoRouterRequestInput<'a> {
    let model = Some("selected");
    let model_id = Some("d-sel");
    let issued_at = moment();
    AutoRouterRequestInput {
        model,
        custom_llm_provider: None,
        routing_decision: Some(decision),
        usage: Some(usage),
        model_id,
        cost_breakdown: None,
        baseline_usage: None,
        baseline_provenance: None,
        moment: issued_at,
        database,
    }
}

#[tokio::test]
async fn request_savings_subtracts_the_classifier() {
    let usage = request_usage(10);
    let database = zone_db();
    let decision = parse_decision(
        r#"{"savings_baseline_model": "baseline", "savings_baseline_deployment_id": "d-base",
            "classifier_cost": 0.5}"#,
    );
    let input = request_input(&usage, &decision, &database);
    let savings = autorouter_savings_for_request(&request_context(), &input).await;
    assert_eq!(savings, Some(decimal("9.5")));
}

#[tokio::test]
async fn request_savings_reads_the_public_baseline_without_an_id() {
    let usage = request_usage(10);
    let database = zone_db();
    let decision = parse_decision(r#"{"savings_baseline_model": "baseline"}"#);
    let input = request_input(&usage, &decision, &database);
    let savings = autorouter_savings_for_request(&request_context(), &input).await;
    assert_eq!(savings, Some(decimal("20")));
}

#[tokio::test]
async fn request_savings_ignores_a_non_numeric_classifier() {
    let usage = request_usage(10);
    let database = zone_db();
    let decision = parse_decision(
        r#"{"savings_baseline_model": "baseline", "savings_baseline_deployment_id": "d-base",
            "classifier_cost": "x"}"#,
    );
    let input = request_input(&usage, &decision, &database);
    let savings = autorouter_savings_for_request(&request_context(), &input).await;
    assert_eq!(savings, Some(decimal("10")));
}

#[tokio::test]
async fn request_savings_requires_usage_model_and_baseline_model() {
    let usage = request_usage(10);
    let database = zone_db();
    let decision = parse_decision(r#"{"savings_baseline_model": "baseline"}"#);
    let context = request_context();
    let mut input = request_input(&usage, &decision, &database);
    input.usage = None;
    assert_eq!(autorouter_savings_for_request(&context, &input).await, None);
    let mut input = request_input(&usage, &decision, &database);
    input.model = Some("");
    assert_eq!(autorouter_savings_for_request(&context, &input).await, None);
    let mut input = request_input(&usage, &decision, &database);
    input.routing_decision = None;
    assert_eq!(autorouter_savings_for_request(&context, &input).await, None);
    let empty_decision = parse_decision(r#"{"savings_baseline_model": ""}"#);
    let empty_input = request_input(&usage, &empty_decision, &database);
    assert_eq!(
        autorouter_savings_for_request(&context, &empty_input).await,
        None
    );
    let number_decision = parse_decision(r#"{"savings_baseline_model": 5}"#);
    let number_input = request_input(&usage, &number_decision, &database);
    assert_eq!(
        autorouter_savings_for_request(&context, &number_input).await,
        None
    );
    let missing_decision = parse_decision("{}");
    let missing_input = request_input(&usage, &missing_decision, &database);
    assert_eq!(
        autorouter_savings_for_request(&context, &missing_input).await,
        None
    );
}

fn parse_metadata(json: &str) -> LoggingMetadata {
    serde_json::from_str(json).expect("the fixture is logging metadata")
}

fn logging_input<'a>(
    metadata: &'a LoggingMetadata,
    usage: &'a RequestUsage,
    database: &'a TimeZoneDatabase,
) -> LoggingPayloadInput<'a> {
    let issued_at = moment();
    LoggingPayloadInput {
        metadata,
        model: Some("selected"),
        custom_llm_provider: None,
        model_id: Some("d-sel"),
        usage: Some(usage),
        cost_breakdown: None,
        baseline_usage: None,
        baseline_provenance: None,
        moment: issued_at,
        database,
    }
}

const LOGGING_DECISION: &str = r#"{"savings_baseline_model": "baseline",
    "savings_baseline_deployment_id": "d-base", "classifier_cost": 0.5}"#;

#[tokio::test]
async fn logging_payload_returns_none_for_an_internal_origin() {
    let usage = request_usage(10);
    let database = zone_db();
    let raw = format!(
        r#"{{"internal_call_origin": "autorouter_classifier", "routing_decision": {LOGGING_DECISION}}}"#
    );
    let metadata = parse_metadata(&raw);
    let input = logging_input(&metadata, &usage, &database);
    assert_eq!(
        savings_for_logging_payload(&request_context(), &input).await,
        None
    );
}

#[tokio::test]
async fn logging_payload_returns_savings_for_a_mapping_decision() {
    let usage = request_usage(10);
    let database = zone_db();
    let raw = format!(r#"{{"routing_decision": {LOGGING_DECISION}}}"#);
    let metadata = parse_metadata(&raw);
    let input = logging_input(&metadata, &usage, &database);
    assert_eq!(
        savings_for_logging_payload(&request_context(), &input).await,
        Some(decimal("9.5"))
    );
}

#[tokio::test]
async fn logging_payload_returns_savings_for_an_empty_internal_origin() {
    let usage = request_usage(10);
    let database = zone_db();
    let raw = format!(r#"{{"internal_call_origin": "", "routing_decision": {LOGGING_DECISION}}}"#);
    let metadata = parse_metadata(&raw);
    let input = logging_input(&metadata, &usage, &database);
    assert_eq!(
        savings_for_logging_payload(&request_context(), &input).await,
        Some(decimal("9.5"))
    );
}

#[test]
fn logging_metadata_rejects_a_non_text_internal_origin() {
    let metadata: Result<LoggingMetadata, serde_json::Error> =
        serde_json::from_str(r#"{"internal_call_origin": true}"#);
    metadata.unwrap_err();
}

#[tokio::test]
async fn logging_payload_ignores_a_scalar_decision() {
    let usage = request_usage(10);
    let database = zone_db();
    let metadata = parse_metadata(r#"{"routing_decision": "selected"}"#);
    let input = logging_input(&metadata, &usage, &database);
    assert_eq!(
        savings_for_logging_payload(&request_context(), &input).await,
        None
    );
}

#[test]
fn scalar_text_extraction_reads_only_text() {
    let text: Scalar = serde_json::from_str("\"claude\"").expect("text scalar");
    let number: Scalar = serde_json::from_str("5").expect("number scalar");
    let null: Scalar = serde_json::from_str("null").expect("null scalar");
    assert_eq!(super::text_value(Some(&text)), Some("claude"));
    assert_eq!(super::text_value(Some(&number)), None);
    assert_eq!(super::text_value(Some(&null)), None);
    assert_eq!(super::text_value(None), None);
}
