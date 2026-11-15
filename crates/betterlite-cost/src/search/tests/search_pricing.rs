use rstest::rstest;
use rust_decimal::Decimal;

use crate::search::error::SearchCostError;
use crate::search::model_info::SearchModelInfo;

use super::support::{FakeSource, MissingPricing, cost, money};

const TIERED: &str = r#"{
    "input_cost_per_query": 0.9,
    "tiered_pricing": [
        {"max_results_range": [1, 9], "input_cost_per_query": 0.001},
        {"max_results_range": [10, 10], "input_cost_per_query": 0.002},
        {"max_results_range": [11, 25], "input_cost_per_query": 0.003},
        {"max_results_range": [26, 100], "input_cost_per_query": 0.025}
    ]
}"#;

#[rstest]
#[case::default_is_ten_results("{}", "0.002")]
#[case::null_reads_as_default(r#"{"max_results": null}"#, "0.002")]
#[case::lowest_bound_of_first_tier(r#"{"max_results": 1}"#, "0.001")]
#[case::top_of_first_tier(r#"{"max_results": 9}"#, "0.001")]
#[case::single_value_tier(r#"{"max_results": 10}"#, "0.002")]
#[case::bottom_of_third_tier(r#"{"max_results": 11}"#, "0.003")]
#[case::top_of_third_tier(r#"{"max_results": 25}"#, "0.003")]
#[case::bottom_of_last_tier(r#"{"max_results": 26}"#, "0.025")]
#[case::top_of_last_tier(r#"{"max_results": 100}"#, "0.025")]
#[case::above_every_tier_uses_highest(r#"{"max_results": 101}"#, "0.025")]
#[case::below_every_tier_uses_highest(r#"{"max_results": 0}"#, "0.025")]
#[case::negative_uses_highest(r#"{"max_results": -5}"#, "0.025")]
#[tokio::test]
async fn tiered_pricing_selects_the_tier_holding_max_results(
    #[case] optional_params: &str,
    #[case] expected: &str,
) {
    let source = FakeSource::new(&[("exa_ai/search", TIERED)]);
    let result = cost(&source, "exa_ai/search", Some("exa_ai"), 1, optional_params).await;
    assert_eq!(result.unwrap(), (money(expected), Decimal::ZERO));
}

#[tokio::test]
async fn tiered_pricing_multiplies_by_the_query_count() {
    let source = FakeSource::new(&[("exa_ai/search", TIERED)]);
    let result = cost(&source, "exa_ai/search", Some("exa_ai"), 4, "{}").await;
    assert_eq!(result.unwrap(), (money("0.008"), Decimal::ZERO));
}

#[tokio::test]
async fn overlapping_tiers_use_the_first_match() {
    let overlapping = r#"{"tiered_pricing": [
        {"max_results_range": [1, 50], "input_cost_per_query": 0.01},
        {"max_results_range": [40, 100], "input_cost_per_query": 0.02}
    ]}"#;
    let source = FakeSource::new(&[("exa_ai/search", overlapping)]);
    let result = cost(
        &source,
        "exa_ai/search",
        Some("exa_ai"),
        1,
        r#"{"max_results": 45}"#,
    )
    .await;
    assert_eq!(result.unwrap(), (money("0.01"), Decimal::ZERO));
}

#[rstest]
#[case::per_query_rate(r#"{"input_cost_per_query": 0.007}"#, 3, "0.021")]
#[case::unset_rate_is_free("{}", 3, "0")]
#[case::null_rate_is_free(r#"{"input_cost_per_query": null}"#, 3, "0")]
#[case::zero_rate_is_free(r#"{"input_cost_per_query": 0.0}"#, 3, "0")]
#[case::no_queries_cost_nothing(r#"{"input_cost_per_query": 0.007}"#, 0, "0")]
#[case::empty_tier_list_is_flat(
    r#"{"input_cost_per_query": 0.007, "tiered_pricing": []}"#,
    2,
    "0.014"
)]
#[case::null_tier_list_is_flat(
    r#"{"input_cost_per_query": 0.007, "tiered_pricing": null}"#,
    2,
    "0.014"
)]
#[tokio::test]
async fn flat_pricing_uses_the_per_query_rate(
    #[case] record: &str,
    #[case] number_of_queries: u64,
    #[case] expected: &str,
) {
    let source = FakeSource::new(&[("tavily/search", record)]);
    let result = cost(
        &source,
        "tavily/search",
        Some("tavily"),
        number_of_queries,
        r#"{"max_results": 500}"#,
    )
    .await;
    assert_eq!(result.unwrap(), (money(expected), Decimal::ZERO));
}

#[tokio::test]
async fn output_cost_is_zero_even_when_the_record_carries_other_rates() {
    let source = FakeSource::new(&[(
        "tavily/search",
        r#"{"input_cost_per_query": 0.007, "output_cost_per_token": 5, "output_cost_per_query": 9}"#,
    )]);
    let result = cost(&source, "tavily/search", Some("tavily"), 1, "{}").await;
    assert_eq!(result.unwrap().1, Decimal::ZERO);
}

#[rstest]
#[case::with_provider("exa_ai/search", Some("exa_ai"))]
#[case::without_provider("exa_ai/search", None)]
#[case::parallel_name_without_the_provider("parallel_ai/search", None)]
#[tokio::test]
async fn the_lookup_receives_the_model_and_provider(
    #[case] model: &str,
    #[case] provider: Option<&str>,
) {
    let source = FakeSource::new(&[(model, r#"{"input_cost_per_query": 0.5}"#)]);
    let result = cost(&source, model, provider, 2, "{}").await;
    assert_eq!(result.unwrap().0, money("1"));
    assert_eq!(
        source.lookups(),
        vec![(model.to_owned(), provider.map(str::to_owned))]
    );
}

#[tokio::test]
async fn parallel_usage_is_ignored_for_other_providers() {
    let source = FakeSource::new(&[("tavily/search", r#"{"input_cost_per_query": 0.5}"#)]);
    let usage = r#"{"_parallel_ai_usage": [{"name": "sku_search", "count": 9}], "mode": "fast"}"#;
    let result = cost(&source, "tavily/search", Some("tavily"), 2, usage).await;
    assert_eq!(result.unwrap().0, money("1"));
}

#[tokio::test]
async fn a_failed_lookup_names_the_model() {
    let source = FakeSource::new(&[]);
    let error = cost(&source, "tavily/search", Some("tavily"), 1, "{}")
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        SearchCostError::ModelInfo { model, source: MissingPricing(_) } if model == "tavily/search"
    ));
}

#[rstest]
#[case::at_the_limit(1, Ok(Decimal::MAX))]
#[case::past_the_limit(2, Err(()))]
#[tokio::test]
async fn a_total_past_the_decimal_limit_is_an_overflow_error(
    #[case] number_of_queries: u64,
    #[case] expected: Result<Decimal, ()>,
) {
    let source = FakeSource::new(&[]);
    source.set(
        "tavily/search",
        SearchModelInfo {
            input_cost_per_query: Some(Decimal::MAX),
            tiered_pricing: None,
        },
    );
    let result = cost(
        &source,
        "tavily/search",
        Some("tavily"),
        number_of_queries,
        "{}",
    )
    .await;
    let summary = match result {
        Ok((input_cost, _)) => Ok(input_cost),
        Err(SearchCostError::Overflow) => Err(()),
        Err(SearchCostError::ModelInfo { .. }) => panic!("the lookup succeeds"),
    };
    assert_eq!(summary, expected);
}

#[tokio::test]
async fn a_mutated_model_info_shows_in_the_next_cost() {
    let source = FakeSource::new(&[("tavily/search", r#"{"input_cost_per_query": 0.5}"#)]);
    let before = cost(&source, "tavily/search", Some("tavily"), 1, "{}").await;
    source.set(
        "tavily/search",
        SearchModelInfo {
            input_cost_per_query: Some(money("0.75")),
            tiered_pricing: None,
        },
    );
    let after = cost(&source, "tavily/search", Some("tavily"), 1, "{}").await;
    assert_eq!(before.unwrap().0, money("0.5"));
    assert_eq!(after.unwrap().0, money("0.75"));
}
