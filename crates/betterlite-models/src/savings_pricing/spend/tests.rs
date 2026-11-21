#![cfg(test)]

use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;

use crate::savings_evidence::billed_at::BilledAt;
use crate::savings_evidence::dynamic::MappingOrScalar;
use crate::savings_evidence::metadata::AutorouterEstimate;
use crate::savings_evidence::usage::{LoggedUsage, PromptTokensDetails};
use crate::savings_pricing::autorouter::AutoRouterDecision;
use crate::savings_pricing::tests::{TestContext, decimal, record};
use crate::savings_pricing::usage::RequestUsage;
use crate::tiered_pricing::Scalar;

use super::{SavingsSpendInput, compute_savings_spend};

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

fn cache_usage(cached: i64) -> RequestUsage {
    let mut usage = request_usage(10);
    usage.usage.cache_read_input_tokens = cached;
    usage.usage.prompt_tokens_details = Some(PromptTokensDetails {
        cached_tokens: cached,
        cache_creation_tokens: 0,
        cache_write_tokens: 0,
        text_tokens: 10_i64
            .checked_sub(cached)
            .expect("the fixture keeps the text tokens nonnegative"),
        audio_tokens: 0,
        image_tokens: 0,
        video_tokens: 0,
        cache_creation_token_details: None,
    });
    usage
}

fn context() -> TestContext {
    TestContext::new()
        .with_identity("claude", None, ("claude", "anthropic"))
        .with_identity("baseline", None, ("baseline", "anthropic"))
        .with_public(
            "claude",
            "anthropic",
            record(
                r#"{"input_cost_per_token": 2.0, "cache_read_input_token_cost": 0.5,
                    "cache_creation_input_token_cost": 2.0}"#,
            ),
        )
        .with_deployment("d-sel", record(r#"{"input_cost_per_token": 1.0}"#))
        .with_deployment("d-base", record(r#"{"input_cost_per_token": 2.0}"#))
}

fn spend_input<'input>(
    usage: &'input RequestUsage,
    billed_at: &'input BilledAt,
    database: &'input TimeZoneDatabase,
) -> SavingsSpendInput<'input> {
    SavingsSpendInput {
        model: Some("claude"),
        custom_llm_provider: None,
        compression_saved_tokens: 0,
        gateway_injected_cache: false,
        routing_decision: None,
        usage: Some(usage),
        model_id: None,
        cost_breakdown: None,
        recorded_autorouter_savings: None,
        recorded_estimate: None,
        billed_at,
        fallback_moment: moment(),
        database,
    }
}

fn scalar(text: &str) -> Scalar {
    serde_json::from_str(text).expect("the fixture is a scalar")
}

fn parse_estimate(json: &str) -> MappingOrScalar<AutorouterEstimate> {
    serde_json::from_str(json).expect("the fixture is an estimate")
}

fn decision(json: &str) -> AutoRouterDecision {
    serde_json::from_str(json).expect("the fixture is a routing decision")
}

#[tokio::test]
async fn compression_prices_only_the_saved_tokens() {
    let usage = request_usage(10);
    let billed_at = BilledAt::Missing;
    let database = zone_db();
    let mut input = spend_input(&usage, &billed_at, &database);
    input.compression_saved_tokens = 4;
    let spend = compute_savings_spend(&context(), &input).await;
    assert_eq!(spend.compression, decimal("8"));
    assert_eq!(spend.prompt_caching, decimal("0"));
    assert_eq!(spend.autorouter, decimal("0"));
}

#[tokio::test]
async fn negative_compression_counts_as_zero() {
    let usage = request_usage(10);
    let billed_at = BilledAt::Missing;
    let database = zone_db();
    let mut input = spend_input(&usage, &billed_at, &database);
    input.compression_saved_tokens = -3;
    let spend = compute_savings_spend(&context(), &input).await;
    assert_eq!(spend.compression, decimal("0"));
}

#[tokio::test]
async fn unavailable_pricing_zeroes_compression() {
    let usage = request_usage(10);
    let billed_at = BilledAt::Missing;
    let database = zone_db();
    let mut input = spend_input(&usage, &billed_at, &database);
    input.compression_saved_tokens = 4;
    let spend = compute_savings_spend(&TestContext::new(), &input).await;
    assert_eq!(spend.compression, decimal("0"));
}

#[tokio::test]
async fn gateway_component_carries_only_the_attributed_subset() {
    let usage = cache_usage(4);
    let billed_at = BilledAt::Missing;
    let database = zone_db();
    let input = spend_input(&usage, &billed_at, &database);
    let spend = compute_savings_spend(&context(), &input).await;
    assert_eq!(spend.prompt_caching, decimal("6"));
    assert_eq!(spend.gateway_injected_caching, decimal("0"));
    let mut attributed = spend_input(&usage, &billed_at, &database);
    attributed.gateway_injected_cache = true;
    let attributed_spend = compute_savings_spend(&context(), &attributed).await;
    assert_eq!(attributed_spend.gateway_injected_caching, decimal("6"));
}

#[tokio::test]
async fn recorded_savings_win_before_usage_validation() {
    let usage = request_usage(10);
    let billed_at = BilledAt::Missing;
    let database = zone_db();
    let recorded = scalar("5");
    let mut input = spend_input(&usage, &billed_at, &database);
    input.usage = None;
    input.recorded_autorouter_savings = Some(&recorded);
    let spend = compute_savings_spend(&context(), &input).await;
    assert_eq!(spend.autorouter, decimal("5"));
}

#[tokio::test]
async fn an_unknown_estimate_suppresses_recomputation() {
    let usage = request_usage(10);
    let billed_at = BilledAt::Missing;
    let database = zone_db();
    let recorded = scalar("5");
    let estimate = parse_estimate(r#"{"version": 4, "status": "estimated"}"#);
    let routing = decision(
        r#"{"savings_baseline_model": "baseline", "savings_baseline_deployment_id": "d-base"}"#,
    );
    let mut input = spend_input(&usage, &billed_at, &database);
    input.model_id = Some("d-sel");
    input.routing_decision = Some(&routing);
    input.recorded_autorouter_savings = Some(&recorded);
    input.recorded_estimate = Some(&estimate);
    let spend = compute_savings_spend(&context(), &input).await;
    assert_eq!(spend.autorouter, decimal("0"));
}

#[tokio::test]
async fn a_versioned_estimate_wins_over_recomputation() {
    let usage = request_usage(10);
    let billed_at = BilledAt::Missing;
    let database = zone_db();
    let recorded = scalar("5");
    let estimate = parse_estimate(r#"{"version": 2, "status": "estimated"}"#);
    let mut input = spend_input(&usage, &billed_at, &database);
    input.recorded_autorouter_savings = Some(&recorded);
    input.recorded_estimate = Some(&estimate);
    let spend = compute_savings_spend(&context(), &input).await;
    assert_eq!(spend.autorouter, decimal("5"));
}

#[tokio::test]
async fn a_versioned_estimate_without_an_amount_is_zero() {
    let usage = request_usage(10);
    let billed_at = BilledAt::Missing;
    let database = zone_db();
    let estimate = parse_estimate(r#"{"version": 2, "status": "estimated"}"#);
    let routing = decision(
        r#"{"savings_baseline_model": "baseline", "savings_baseline_deployment_id": "d-base"}"#,
    );
    let mut input = spend_input(&usage, &billed_at, &database);
    input.model_id = Some("d-sel");
    input.routing_decision = Some(&routing);
    input.recorded_estimate = Some(&estimate);
    let spend = compute_savings_spend(&context(), &input).await;
    assert_eq!(spend.autorouter, decimal("0"));
}

#[tokio::test]
async fn request_savings_fill_the_autorouter_without_recorded_values() {
    let usage = request_usage(10);
    let billed_at = BilledAt::Missing;
    let database = zone_db();
    let routing = decision(
        r#"{"savings_baseline_model": "baseline", "savings_baseline_deployment_id": "d-base"}"#,
    );
    let mut input = spend_input(&usage, &billed_at, &database);
    input.model_id = Some("d-sel");
    input.routing_decision = Some(&routing);
    let spend = compute_savings_spend(&context(), &input).await;
    assert_eq!(spend.autorouter, decimal("10"));
}

#[tokio::test]
async fn unavailable_autorouter_is_zero() {
    let usage = request_usage(10);
    let billed_at = BilledAt::Missing;
    let database = zone_db();
    let input = spend_input(&usage, &billed_at, &database);
    let spend = compute_savings_spend(&context(), &input).await;
    assert_eq!(spend.autorouter, decimal("0"));
}
