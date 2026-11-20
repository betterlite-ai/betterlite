#![cfg(test)]

use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;

use crate::cost_usage::CompletionDetails;
use crate::cost_usage::modality_projection::CacheCreationTokenDetails;
use crate::savings_evidence::error::SavingsEvidenceError;
use crate::savings_evidence::numeric::RoutingDecision;
use crate::savings_evidence::pricing_basis::PricingBasis;
use crate::savings_evidence::snapshot::{BaselineCostSnapshot, CostBreakdown};
use crate::savings_evidence::spend::BaselineCosts;
use crate::savings_evidence::usage::{LoggedUsage, PromptTokensDetails};
use crate::savings_pricing::context::ModelIdentity;
use crate::savings_pricing::tests::{TestContext, decimal, priced, record};
use crate::savings_pricing::usage::RequestUsage;

use super::{
    ComparisonInput, ComparisonProvenance, CostOfUsageInput, SnapshotInput, amounts_close,
    baseline_cost_snapshot, cost_of_usage, price_baseline_comparison, same_prices, same_usage,
};

fn moment() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("the fixture timestamp is valid")
}

fn database() -> TimeZoneDatabase {
    TimeZoneDatabase::from_env()
}

fn identity(model: &str, provider: &str) -> ModelIdentity {
    ModelIdentity {
        model: model.to_owned(),
        provider: provider.to_owned(),
    }
}

fn request_usage(prompt_tokens: i64, completion_tokens: i64) -> RequestUsage {
    RequestUsage {
        usage: LoggedUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens
                .checked_add(completion_tokens)
                .expect("the fixture total fits"),
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
            prompt_tokens_details: None,
            completion_tokens_details: None,
        },
        cache_creation_token_details: None,
    }
}

fn detailed_usage() -> RequestUsage {
    RequestUsage {
        usage: LoggedUsage {
            prompt_tokens: 10,
            completion_tokens: 4,
            total_tokens: 14,
            cache_read_input_tokens: 2,
            cache_creation_input_tokens: 3,
            prompt_tokens_details: Some(PromptTokensDetails {
                cached_tokens: 2,
                cache_creation_tokens: 3,
                cache_write_tokens: 3,
                text_tokens: 5,
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

fn snapshot_json(json: &str) -> BaselineCostSnapshot {
    serde_json::from_str(json).expect("the fixture is a snapshot")
}

#[test]
fn cost_of_usage_prices_anthropic_with_a_supplied_record() {
    let context = TestContext::new();
    let usage = request_usage(10, 4);
    let info = priced(&record(
        r#"{"input_cost_per_token": 0.5, "output_cost_per_token": 2.0}"#,
    ));
    let provider = identity("claude", "anthropic");
    let total = cost_of_usage(
        &context,
        &CostOfUsageInput {
            identity: &provider,
            usage: &usage,
            model_info: Some(&info),
            basis: &PricingBasis::default(),
            moment: moment(),
            database: &database(),
        },
    )
    .expect("the supplied record prices the usage");
    assert_eq!(total, decimal("13"));
}

#[test]
fn cost_of_usage_prices_a_generic_provider() {
    let context = TestContext::new();
    let usage = request_usage(10, 4);
    let info = priced(&record(
        r#"{"input_cost_per_token": 0.5, "output_cost_per_token": 1.0}"#,
    ));
    let provider = identity("gpt-4o", "openai");
    let total = cost_of_usage(
        &context,
        &CostOfUsageInput {
            identity: &provider,
            usage: &usage,
            model_info: Some(&info),
            basis: &PricingBasis::default(),
            moment: moment(),
            database: &database(),
        },
    )
    .expect("the supplied record prices the usage");
    assert_eq!(total, decimal("9"));
}

#[test]
fn cost_of_usage_uses_public_prices_without_a_supplied_record() {
    let context = TestContext::new().with_public(
        "gpt-4o",
        "openai",
        record(r#"{"input_cost_per_token": 0.5, "output_cost_per_token": 1.0}"#),
    );
    let usage = request_usage(10, 4);
    let provider = identity("gpt-4o", "openai");
    let total = cost_of_usage(
        &context,
        &CostOfUsageInput {
            identity: &provider,
            usage: &usage,
            model_info: None,
            basis: &PricingBasis::default(),
            moment: moment(),
            database: &database(),
        },
    )
    .expect("the public entry prices the usage");
    assert_eq!(total, decimal("9"));
}

#[test]
fn cost_of_usage_is_none_without_any_pricing() {
    let context = TestContext::new();
    let usage = request_usage(10, 4);
    let provider = identity("unmapped", "openai");
    assert_eq!(
        cost_of_usage(
            &context,
            &CostOfUsageInput {
                identity: &provider,
                usage: &usage,
                model_info: None,
                basis: &PricingBasis::default(),
                moment: moment(),
                database: &database(),
            },
        ),
        None
    );
}

#[test]
fn snapshot_records_provider_basis_and_token_cost() {
    let breakdown: CostBreakdown =
        serde_json::from_str(r#"{"service_tier": "batch", "input_cost": 1.0, "output_cost": 0.5}"#)
            .expect("the fixture is a breakdown");
    let decision: RoutingDecision =
        serde_json::from_str(r#"{"classifier_cost": 0.25}"#).expect("the fixture is a decision");
    let snapshot = baseline_cost_snapshot(&SnapshotInput {
        model: "claude",
        prices: None,
        actual_spend: decimal("10"),
        cost_breakdown: Some(&breakdown),
        routing_decision: Some(&decision),
    })
    .expect("the recorded evidence is valid");
    assert_eq!(snapshot.provider, "anthropic");
    assert_eq!(snapshot.basis.service_tier.as_deref(), Some("batch"));
    assert_eq!(snapshot.actual_token_cost, Some(decimal("1.5")));
    assert_eq!(snapshot.classifier_cost, decimal("0.25"));
    assert!(snapshot.prices.is_none());
}

#[test]
fn snapshot_defaults_the_classifier_to_zero() {
    let snapshot = baseline_cost_snapshot(&SnapshotInput {
        model: "claude",
        prices: None,
        actual_spend: decimal("10"),
        cost_breakdown: None,
        routing_decision: None,
    })
    .expect("the recorded evidence is valid");
    assert_eq!(snapshot.classifier_cost, decimal("0"));
    assert_eq!(snapshot.actual_token_cost, None);
}

#[test]
fn snapshot_rejects_a_negative_spend() {
    let result = baseline_cost_snapshot(&SnapshotInput {
        model: "claude",
        prices: None,
        actual_spend: decimal("-1"),
        cost_breakdown: None,
        routing_decision: None,
    });
    assert!(matches!(
        result,
        Err(SavingsEvidenceError::NegativeAmount {
            field: "actual_spend"
        })
    ));
}

#[test]
fn snapshot_rejects_a_negative_classifier() {
    let decision: RoutingDecision =
        serde_json::from_str(r#"{"classifier_cost": -2}"#).expect("the fixture is a decision");
    let result = baseline_cost_snapshot(&SnapshotInput {
        model: "claude",
        prices: None,
        actual_spend: decimal("1"),
        cost_breakdown: None,
        routing_decision: Some(&decision),
    });
    assert!(matches!(
        result,
        Err(SavingsEvidenceError::NegativeAmount {
            field: "classifier_cost"
        })
    ));
}

fn modeled_snapshot(token_cost: &str) -> BaselineCostSnapshot {
    snapshot_json(&format!(
        r#"{{"model": "claude", "provider": "anthropic",
            "prices": {{"input_cost_per_token": 0.5, "output_cost_per_token": 1.0}},
            "actual_spend": 10, "actual_token_cost": {token_cost}, "classifier_cost": 1}}"#
    ))
}

#[test]
fn comparison_requires_usage_and_provenance() {
    let snapshot = modeled_snapshot("3");
    let usage = request_usage(10, 4);
    assert_eq!(
        price_baseline_comparison(
            &TestContext::new(),
            &ComparisonInput {
                snapshot: &snapshot,
                baseline_usage: None,
                provenance: Some(ComparisonProvenance::ObservedIdentical),
                moment: moment(),
                database: &database(),
            },
        ),
        None
    );
    assert_eq!(
        price_baseline_comparison(
            &TestContext::new(),
            &ComparisonInput {
                snapshot: &snapshot,
                baseline_usage: Some(&usage),
                provenance: None,
                moment: moment(),
                database: &database(),
            },
        ),
        None
    );
}

#[test]
fn observed_identical_keeps_the_classifier_on_the_actual_side() {
    let snapshot = modeled_snapshot("3");
    let usage = request_usage(10, 4);
    let costs = price_baseline_comparison(
        &TestContext::new(),
        &ComparisonInput {
            snapshot: &snapshot,
            baseline_usage: Some(&usage),
            provenance: Some(ComparisonProvenance::ObservedIdentical),
            moment: moment(),
            database: &database(),
        },
    )
    .expect("the comparison prices");
    assert_eq!(
        costs,
        BaselineCosts {
            actual: decimal("11"),
            baseline: decimal("10"),
        }
    );
}

#[test]
fn modeled_replaces_only_the_token_portion() {
    let snapshot = modeled_snapshot("3");
    let usage = request_usage(10, 4);
    let costs = price_baseline_comparison(
        &TestContext::new(),
        &ComparisonInput {
            snapshot: &snapshot,
            baseline_usage: Some(&usage),
            provenance: Some(ComparisonProvenance::Modeled),
            moment: moment(),
            database: &database(),
        },
    )
    .expect("the comparison prices");
    assert_eq!(
        costs,
        BaselineCosts {
            actual: decimal("11"),
            baseline: decimal("16"),
        }
    );
}

#[test]
fn modeled_requires_prices_and_a_recorded_token_cost() {
    let unpriced = snapshot_json(
        r#"{"model": "claude", "provider": "anthropic", "actual_spend": 10, "actual_token_cost": 3}"#,
    );
    let unrecorded = snapshot_json(
        r#"{"model": "claude", "provider": "anthropic",
            "prices": {"input_cost_per_token": 0.5}, "actual_spend": 10}"#,
    );
    let usage = request_usage(10, 4);
    assert_eq!(modeled_costs(&unpriced, &usage), None);
    assert_eq!(modeled_costs(&unrecorded, &usage), None);
}

fn modeled_costs(snapshot: &BaselineCostSnapshot, usage: &RequestUsage) -> Option<BaselineCosts> {
    price_baseline_comparison(
        &TestContext::new(),
        &ComparisonInput {
            snapshot,
            baseline_usage: Some(usage),
            provenance: Some(ComparisonProvenance::Modeled),
            moment: moment(),
            database: &database(),
        },
    )
}

#[test]
fn modeled_keeps_a_zero_token_cost_and_a_zero_baseline() {
    let snapshot = snapshot_json(
        r#"{"model": "claude", "provider": "anthropic",
            "prices": {"input_cost_per_token": 0.5}, "actual_spend": 10, "actual_token_cost": 10}"#,
    );
    let usage = request_usage(0, 0);
    assert_eq!(
        modeled_costs(&snapshot, &usage),
        Some(BaselineCosts {
            actual: decimal("10"),
            baseline: decimal("0"),
        })
    );
}

#[test]
fn cost_of_usage_keeps_anthropic_out_of_residency_and_location() {
    let context = TestContext::new();
    let usage = request_usage(10, 0);
    let info = priced(&record(
        r#"{"input_cost_per_token": 1.0, "regional_endpoint_uplift_multiplier": 3}"#,
    ));
    let basis = PricingBasis {
        service_tier: None,
        data_residency: None,
        vertex_location: Some(String::from("us-east5")),
    };
    let anthropic = identity("claude", "anthropic");
    let total = cost_of_usage(
        &context,
        &CostOfUsageInput {
            identity: &anthropic,
            usage: &usage,
            model_info: Some(&info),
            basis: &basis,
            moment: moment(),
            database: &database(),
        },
    )
    .expect("the anthropic rates price the usage");
    assert_eq!(total, decimal("10"));
    let generic = identity("gpt-4o", "openai");
    let generic_total = cost_of_usage(
        &context,
        &CostOfUsageInput {
            identity: &generic,
            usage: &usage,
            model_info: Some(&info),
            basis: &basis,
            moment: moment(),
            database: &database(),
        },
    )
    .expect("the generic rates apply the location uplift");
    assert_eq!(generic_total, decimal("30"));
}

#[test]
fn modeled_rejects_a_negative_token_cost() {
    let snapshot = snapshot_json(
        r#"{"model": "claude", "provider": "anthropic",
            "prices": {"input_cost_per_token": -1.0}, "actual_spend": 10, "actual_token_cost": 0}"#,
    );
    let usage = request_usage(10, 4);
    assert_eq!(
        price_baseline_comparison(
            &TestContext::new(),
            &ComparisonInput {
                snapshot: &snapshot,
                baseline_usage: Some(&usage),
                provenance: Some(ComparisonProvenance::Modeled),
                moment: moment(),
                database: &database(),
            },
        ),
        None
    );
}

#[test]
fn modeled_rejects_a_negative_baseline_total() {
    let snapshot = snapshot_json(
        r#"{"model": "claude", "provider": "anthropic",
            "prices": {"input_cost_per_token": 0.5}, "actual_spend": 0, "actual_token_cost": 3}"#,
    );
    let usage = request_usage(0, 0);
    assert_eq!(
        price_baseline_comparison(
            &TestContext::new(),
            &ComparisonInput {
                snapshot: &snapshot,
                baseline_usage: Some(&usage),
                provenance: Some(ComparisonProvenance::Modeled),
                moment: moment(),
                database: &database(),
            },
        ),
        None
    );
}

#[test]
fn same_prices_compares_flat_values() {
    let one = record(r#"{"input_cost_per_token": 1e-6, "output_cost_per_token": 2}"#);
    let equivalent = record(r#"{"input_cost_per_token": 0.000001, "output_cost_per_token": 2.0}"#);
    let different = record(r#"{"input_cost_per_token": 1e-5, "output_cost_per_token": 2}"#);
    let missing = record(r#"{"input_cost_per_token": 1e-6}"#);
    let nested = record(r#"{"tiered_pricing": [{"input_cost_per_token": 1e-6}]}"#);
    let superset =
        record(r#"{"input_cost_per_token": 1e-6, "output_cost_per_token": 2, "mode": "chat"}"#);
    assert!(!same_prices(Some(&one), Some(&superset)));
    assert!(!same_prices(Some(&superset), Some(&one)));
    assert!(same_prices(Some(&one), Some(&equivalent)));
    assert!(!same_prices(Some(&one), Some(&different)));
    assert!(!same_prices(Some(&one), Some(&missing)));
    assert!(!same_prices(Some(&one), None));
    assert!(!same_prices(None, Some(&one)));
    assert!(same_prices(None, None));
    assert!(!same_prices(Some(&nested), Some(&nested)));
}

#[test]
fn same_prices_compares_bool_text_and_null_values() {
    let one = record(r#"{"supports_vision": true, "mode": "chat", "note": null}"#);
    let equivalent = record(r#"{"supports_vision": true, "mode": "chat", "note": null}"#);
    let different_bool = record(r#"{"supports_vision": false, "mode": "chat", "note": null}"#);
    let different_text = record(r#"{"supports_vision": true, "mode": "embedding", "note": null}"#);
    let different_null = record(r#"{"supports_vision": true, "mode": "chat", "note": "x"}"#);
    assert!(same_prices(Some(&one), Some(&equivalent)));
    assert!(!same_prices(Some(&one), Some(&different_bool)));
    assert!(!same_prices(Some(&one), Some(&different_text)));
    assert!(!same_prices(Some(&one), Some(&different_null)));
}

#[test]
fn same_prices_compares_unparsable_numbers_by_lexeme() {
    let one = record(r#"{"input_cost_per_token": 1e308}"#);
    let equivalent = record(r#"{"input_cost_per_token": 1e308}"#);
    assert!(same_prices(Some(&one), Some(&equivalent)));
}

#[test]
fn same_usage_compares_every_field() {
    let base = detailed_usage();
    let mut other = detailed_usage();
    assert!(same_usage(&base, &other));
    other.usage.prompt_tokens = 11;
    assert!(!same_usage(&base, &other));
    let mut other_completion = detailed_usage();
    other_completion.usage.completion_tokens_details = Some(CompletionDetails::default());
    assert!(!same_usage(&base, &other_completion));
    let mut other_prompt = detailed_usage();
    if let Some(details) = other_prompt.usage.prompt_tokens_details.as_mut() {
        details.cached_tokens = 9;
    }
    assert!(!same_usage(&base, &other_prompt));
    let mut other_ttl = detailed_usage();
    other_ttl.cache_creation_token_details = Some(CacheCreationTokenDetails::default());
    assert!(!same_usage(&base, &other_ttl));
}

#[test]
fn amounts_close_keeps_the_source_tolerance() {
    assert!(amounts_close(decimal("1"), decimal("1.000000001")));
    assert!(!amounts_close(decimal("1"), decimal("1.000002")));
    assert!(amounts_close(decimal("0"), decimal("0.000000000001")));
    assert!(!amounts_close(decimal("0"), decimal("0.00000000001")));
    assert!(!amounts_close(decimal("1"), decimal("2")));
}
