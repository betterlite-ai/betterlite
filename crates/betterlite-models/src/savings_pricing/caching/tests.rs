#![cfg(test)]

use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;

use crate::cost_usage::modality_projection::CacheCreationTokenDetails;
use crate::savings_evidence::billed_at::BilledAt;
use crate::savings_evidence::usage::{LoggedUsage, PromptTokensDetails};
use crate::savings_pricing::tests::{TestContext, decimal, number, priced, record};
use crate::savings_pricing::usage::RequestUsage;

use super::{
    PromptCachingForRequest, PromptCachingInput, RequestPricingInput, prompt_caching_savings,
    prompt_caching_savings_for_request, request_savings_pricing,
};

fn moment() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("the fixture timestamp is valid")
}

fn database() -> TimeZoneDatabase {
    TimeZoneDatabase::from_env()
}

fn cache_usage(cached: i64, written: i64, ttl: CacheCreationTokenDetails) -> RequestUsage {
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
                ..PromptTokensDetails::default()
            }),
            completion_tokens_details: None,
        },
        cache_creation_token_details: Some(ttl),
    }
}

fn plain_usage() -> RequestUsage {
    RequestUsage {
        usage: LoggedUsage {
            prompt_tokens: 10,
            completion_tokens: 0,
            total_tokens: 10,
            ..LoggedUsage::default()
        },
        cache_creation_token_details: None,
    }
}

#[tokio::test]
async fn request_savings_pricing_prefers_the_deployment_mapping() {
    let context = TestContext::new()
        .with_identity("claude", None, ("claude", "anthropic"))
        .with_public(
            "claude",
            "anthropic",
            record(r#"{"input_cost_per_token": 1.0}"#),
        )
        .with_deployment("d1", record(r#"{"input_cost_per_token": 2.0}"#));
    let pricing = request_savings_pricing(
        &context,
        &RequestPricingInput {
            model: Some("claude"),
            custom_llm_provider: None,
            model_id: Some("d1"),
        },
    )
    .await;
    assert_eq!(pricing.provider.as_deref(), Some("anthropic"));
    assert_eq!(
        pricing
            .pricing
            .as_ref()
            .and_then(|priced| number(&priced.record, "input_cost_per_token")),
        Some(decimal("2"))
    );
}

#[tokio::test]
async fn request_savings_pricing_falls_back_from_an_empty_deployment_mapping() {
    let context = TestContext::new()
        .with_identity("claude", None, ("claude", "anthropic"))
        .with_public(
            "claude",
            "anthropic",
            record(r#"{"input_cost_per_token": 1.0}"#),
        )
        .with_deployment("d1", record("{}"));
    let pricing = request_savings_pricing(
        &context,
        &RequestPricingInput {
            model: Some("claude"),
            custom_llm_provider: None,
            model_id: Some("d1"),
        },
    )
    .await;
    assert_eq!(
        pricing
            .pricing
            .as_ref()
            .and_then(|priced| number(&priced.record, "input_cost_per_token")),
        Some(decimal("1"))
    );
}

#[tokio::test]
async fn request_savings_pricing_uses_the_raw_provider_without_an_identity() {
    let context = TestContext::new();
    let pricing = request_savings_pricing(
        &context,
        &RequestPricingInput {
            model: Some("unmapped"),
            custom_llm_provider: Some("custom"),
            model_id: None,
        },
    )
    .await;
    assert_eq!(pricing.provider.as_deref(), Some("custom"));
    assert!(pricing.pricing.is_none());
}

#[tokio::test]
async fn request_savings_pricing_prefers_the_canonical_provider() {
    let context =
        TestContext::new().with_identity("claude", Some("alias"), ("claude", "anthropic"));
    let pricing = request_savings_pricing(
        &context,
        &RequestPricingInput {
            model: Some("claude"),
            custom_llm_provider: Some("alias"),
            model_id: None,
        },
    )
    .await;
    assert_eq!(pricing.provider.as_deref(), Some("anthropic"));
}

#[test]
fn prompt_caching_savings_requires_pricing_and_usage() {
    let usage = plain_usage();
    let pricing = priced(&record(r#"{"input_cost_per_token": 2.0}"#));
    assert_eq!(
        prompt_caching_savings(&PromptCachingInput {
            pricing: None,
            provider: None,
            usage: Some(&usage),
            cost_breakdown: None,
            billed_at: &BilledAt::Missing,
            fallback_moment: moment(),
            database: &database(),
        }),
        None
    );
    assert_eq!(
        prompt_caching_savings(&PromptCachingInput {
            pricing: Some(&pricing),
            provider: None,
            usage: None,
            cost_breakdown: None,
            billed_at: &BilledAt::Missing,
            fallback_moment: moment(),
            database: &database(),
        }),
        None
    );
}

#[test]
fn prompt_caching_savings_prices_the_read_discount() {
    let usage = cache_usage(4, 0, CacheCreationTokenDetails::default());
    let pricing = priced(&record(
        r#"{"input_cost_per_token": 2.0, "cache_read_input_token_cost": 0.5}"#,
    ));
    let savings = prompt_caching_savings(&PromptCachingInput {
        pricing: Some(&pricing),
        provider: Some("anthropic"),
        usage: Some(&usage),
        cost_breakdown: None,
        billed_at: &BilledAt::Timestamp(moment()),
        fallback_moment: moment(),
        database: &database(),
    })
    .expect("the cache read is priced");
    assert_eq!(savings, decimal("6"));
}

#[test]
fn prompt_caching_savings_charges_the_write_premium() {
    let usage = cache_usage(0, 4, CacheCreationTokenDetails::default());
    let pricing = priced(&record(
        r#"{"input_cost_per_token": 1.0, "cache_creation_input_token_cost": 2.0}"#,
    ));
    let savings = prompt_caching_savings(&PromptCachingInput {
        pricing: Some(&pricing),
        provider: Some("anthropic"),
        usage: Some(&usage),
        cost_breakdown: None,
        billed_at: &BilledAt::Timestamp(moment()),
        fallback_moment: moment(),
        database: &database(),
    })
    .expect("the cache write is priced");
    assert_eq!(savings, decimal("-4"));
}

#[test]
fn prompt_caching_savings_uses_the_one_hour_write_rate() {
    let usage = cache_usage(
        0,
        4,
        CacheCreationTokenDetails {
            ephemeral_5m_input_tokens: None,
            ephemeral_1h_input_tokens: Some(4),
        },
    );
    let pricing = priced(&record(
        r#"{"input_cost_per_token": 1.0, "cache_creation_input_token_cost": 2.0,
            "cache_creation_input_token_cost_above_1hr": 4.0}"#,
    ));
    let savings = prompt_caching_savings(&PromptCachingInput {
        pricing: Some(&pricing),
        provider: Some("anthropic"),
        usage: Some(&usage),
        cost_breakdown: None,
        billed_at: &BilledAt::Timestamp(moment()),
        fallback_moment: moment(),
        database: &database(),
    })
    .expect("the cache write is priced");
    assert_eq!(savings, decimal("-12"));
}

#[test]
fn prompt_caching_savings_prices_an_unparseable_billed_at_at_the_fallback() {
    let usage = cache_usage(4, 0, CacheCreationTokenDetails::default());
    let pricing = priced(&record(
        r#"{"input_cost_per_token": 2.0, "cache_read_input_token_cost": 0.5}"#,
    ));
    let savings = prompt_caching_savings(&PromptCachingInput {
        pricing: Some(&pricing),
        provider: None,
        usage: Some(&usage),
        cost_breakdown: None,
        billed_at: &BilledAt::Text(String::from("not a timestamp")),
        fallback_moment: moment(),
        database: &database(),
    })
    .expect("the cache read is priced");
    assert_eq!(savings, decimal("6"));
}

#[tokio::test]
async fn prompt_caching_savings_for_request_prices_the_public_record() {
    let context = TestContext::new()
        .with_identity("gpt-4o", None, ("gpt-4o", "openai"))
        .with_public(
            "gpt-4o",
            "openai",
            record(r#"{"input_cost_per_token": 2.0, "cache_read_input_token_cost": 0.5}"#),
        );
    let usage = cache_usage(4, 0, CacheCreationTokenDetails::default());
    let savings = prompt_caching_savings_for_request(
        &context,
        &PromptCachingForRequest {
            model: Some("gpt-4o"),
            custom_llm_provider: None,
            usage: Some(&usage),
            model_id: None,
            cost_breakdown: None,
            billed_at: &BilledAt::Missing,
            fallback_moment: moment(),
            database: &database(),
        },
    )
    .await;
    assert_eq!(savings, Some(decimal("6")));
}
