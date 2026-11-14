use rstest::rstest;
use rust_decimal::Decimal;

use crate::search::error::SearchCostError;
use crate::search::model_info::SearchModelInfo;

use super::support::{FakeSource, MissingPricing, cost, money};

const STANDARD: &str = r#"{"input_cost_per_query": 0.005}"#;
const FAST: &str = r#"{"input_cost_per_query": 0.003}"#;
const TURBO: &str = r#"{"input_cost_per_query": 0.002}"#;

fn parallel_source() -> FakeSource {
    FakeSource::new(&[
        ("parallel_ai/search", STANDARD),
        ("parallel_ai/search-fast", FAST),
        ("parallel_ai/search-turbo", TURBO),
    ])
}

#[rstest]
#[case::default_is_basic("{}", "parallel_ai/search", "0.005")]
#[case::basic(r#"{"mode": "basic"}"#, "parallel_ai/search", "0.005")]
#[case::advanced(r#"{"mode": "advanced"}"#, "parallel_ai/search", "0.005")]
#[case::fast(r#"{"mode": "fast"}"#, "parallel_ai/search-fast", "0.003")]
#[case::turbo(r#"{"mode": "turbo"}"#, "parallel_ai/search-turbo", "0.002")]
#[case::unknown_mode(r#"{"mode": "slow"}"#, "parallel_ai/search", "0.005")]
#[case::empty_mode(r#"{"mode": ""}"#, "parallel_ai/search", "0.005")]
#[case::pro_processor(r#"{"processor": "pro"}"#, "parallel_ai/search", "0.005")]
#[case::mode_beats_processor(
    r#"{"processor": "pro", "mode": "fast"}"#,
    "parallel_ai/search-fast",
    "0.003"
)]
#[tokio::test]
async fn parallel_ai_prices_the_model_its_mode_selects(
    #[case] optional_params: &str,
    #[case] pricing_model: &str,
    #[case] expected: &str,
) {
    let source = parallel_source();
    let result = cost(
        &source,
        "parallel_ai/search",
        Some("parallel_ai"),
        1,
        optional_params,
    )
    .await;
    assert_eq!(result.unwrap(), (money(expected), Decimal::ZERO));
    assert_eq!(
        source.lookups(),
        vec![(pricing_model.to_owned(), Some("parallel_ai".to_owned()))]
    );
}

#[rstest]
#[case::default_results(r#"{"max_results": 10}"#, "0.005")]
#[case::one_more_result(r#"{"max_results": 11}"#, "0.006")]
#[case::many_more_results(r#"{"max_results": 25}"#, "0.020")]
#[case::fewer_results_cost_no_less(r#"{"max_results": 5}"#, "0.005")]
#[case::null_results(r#"{"max_results": null}"#, "0.005")]
#[case::negative_results_read_as_default(r#"{"max_results": -3}"#, "0.005")]
#[case::advanced_settings_beat_max_results(
    r#"{"advanced_settings": {"max_results": 20}, "max_results": 12}"#,
    "0.015"
)]
#[case::smaller_advanced_settings_still_win(
    r#"{"advanced_settings": {"max_results": 5}, "max_results": 30}"#,
    "0.005"
)]
#[case::negative_advanced_settings_fall_through(
    r#"{"advanced_settings": {"max_results": -1}, "max_results": 12}"#,
    "0.007"
)]
#[case::empty_advanced_settings_fall_through(
    r#"{"advanced_settings": {}, "max_results": 12}"#,
    "0.007"
)]
#[case::null_advanced_settings_fall_through(
    r#"{"advanced_settings": null, "max_results": 12}"#,
    "0.007"
)]
#[tokio::test]
async fn parallel_ai_charges_each_result_past_ten_without_usage(
    #[case] optional_params: &str,
    #[case] expected: &str,
) {
    let source = parallel_source();
    let result = cost(
        &source,
        "parallel_ai/search",
        Some("parallel_ai"),
        1,
        optional_params,
    )
    .await;
    assert_eq!(result.unwrap().0, money(expected));
}

#[rstest]
#[case::searches_only(r#"[{"name": "sku_search", "count": 2}]"#, "0.010")]
#[case::additional_results_only(
    r#"[{"name": "sku_search_additional_results", "count": 7}]"#,
    "0.012"
)]
#[case::both_skus(
    r#"[{"name": "sku_search", "count": 3}, {"name": "sku_search_additional_results", "count": 4}]"#,
    "0.019"
)]
#[case::repeated_sku_is_summed(
    r#"[{"name": "sku_search", "count": 1}, {"name": "sku_search", "count": 2}]"#,
    "0.015"
)]
#[case::zero_searches_are_zero(r#"[{"name": "sku_search", "count": 0}]"#, "0")]
#[case::zero_additional_results_beat_max_results(
    r#"[{"name": "sku_search_additional_results", "count": 0}]"#,
    "0.005"
)]
#[case::empty_usage_is_still_usage("[]", "0.005")]
#[case::other_sku_counts_for_nothing(r#"[{"name": "other", "count": 9}]"#, "0.005")]
#[case::string_count_is_skipped(r#"[{"name": "sku_search", "count": "2"}]"#, "0.005")]
#[case::negative_count_is_skipped(r#"[{"name": "sku_search", "count": -1}]"#, "0.005")]
#[case::boolean_count_is_skipped(r#"[{"name": "sku_search", "count": true}]"#, "0.005")]
#[case::float_count_is_skipped(r#"[{"name": "sku_search", "count": 2.5}]"#, "0.005")]
#[case::missing_count_is_skipped(r#"[{"name": "sku_search"}]"#, "0.005")]
#[case::numeric_name_is_skipped(r#"[{"name": 5, "count": 2}]"#, "0.005")]
#[case::bad_item_skipped_good_item_kept(
    r#"[{"name": "sku_search", "count": 2}, {"name": "sku_search", "count": "x"}]"#,
    "0.010"
)]
#[tokio::test]
async fn parallel_ai_counts_the_skus_of_valid_usage(#[case] usage: &str, #[case] expected: &str) {
    let source = parallel_source();
    let optional_params = format!(r#"{{"_parallel_ai_usage": {usage}, "max_results": 50}}"#);
    let result = cost(
        &source,
        "parallel_ai/search",
        Some("parallel_ai"),
        1,
        &optional_params,
    )
    .await;
    assert_eq!(result.unwrap().0, money(expected));
}

#[rstest]
#[case::null("null")]
#[case::text(r#""abc""#)]
#[case::numbers("[1, 2]")]
#[case::one_mapping(r#"{"name": "sku_search", "count": 2}"#)]
#[case::mixed_list(r#"[{"name": "sku_search", "count": 2}, 7]"#)]
#[tokio::test]
async fn parallel_ai_falls_back_to_max_results_when_usage_is_invalid(#[case] usage: &str) {
    let source = parallel_source();
    let optional_params = format!(r#"{{"_parallel_ai_usage": {usage}, "max_results": 12}}"#);
    let result = cost(
        &source,
        "parallel_ai/search",
        Some("parallel_ai"),
        1,
        &optional_params,
    )
    .await;
    assert_eq!(result.unwrap().0, money("0.007"));
}

#[tokio::test]
async fn parallel_ai_usage_prices_the_mode_model() {
    let source = parallel_source();
    let optional_params =
        r#"{"mode": "fast", "_parallel_ai_usage": [{"name": "sku_search", "count": 2}]}"#;
    let result = cost(
        &source,
        "parallel_ai/search",
        Some("parallel_ai"),
        1,
        optional_params,
    )
    .await;
    assert_eq!(result.unwrap().0, money("0.006"));
}

#[tokio::test]
async fn parallel_ai_charges_results_past_ten_from_max_results() {
    let source = parallel_source();
    let optional_params = r#"{"max_results": 25}"#;
    let result = cost(
        &source,
        "parallel_ai/search",
        Some("parallel_ai"),
        1,
        optional_params,
    )
    .await;
    assert_eq!(result.unwrap().0, money("0.02"));
}

#[tokio::test]
async fn parallel_ai_ignores_the_query_count_and_the_model_name() {
    let source = parallel_source();
    let result = cost(&source, "anything", Some("parallel_ai"), 5, "{}").await;
    assert_eq!(result.unwrap(), (money("0.005"), Decimal::ZERO));
}

#[tokio::test]
async fn parallel_ai_without_a_listed_price_charges_only_extra_results() {
    let source = FakeSource::new(&[("parallel_ai/search", "{}")]);
    let optional_params = r#"{"max_results": 15}"#;
    let result = cost(
        &source,
        "parallel_ai/search",
        Some("parallel_ai"),
        1,
        optional_params,
    )
    .await;
    assert_eq!(result.unwrap().0, money("0.005"));
}

#[tokio::test]
async fn parallel_ai_lookup_failure_names_the_pricing_model() {
    let source = FakeSource::new(&[]);
    let error = cost(
        &source,
        "parallel_ai/search",
        Some("parallel_ai"),
        1,
        r#"{"mode": "turbo"}"#,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        &error,
        SearchCostError::ModelInfo { model, source: MissingPricing(_) } if model == "parallel_ai/search-turbo"
    ));
}

#[tokio::test]
async fn parallel_ai_reads_the_model_info_on_every_call() {
    let source = parallel_source();
    let before = cost(&source, "parallel_ai/search", Some("parallel_ai"), 1, "{}").await;
    source.set(
        "parallel_ai/search",
        SearchModelInfo {
            input_cost_per_query: Some(money("0.01")),
            tiered_pricing: None,
        },
    );
    let after = cost(&source, "parallel_ai/search", Some("parallel_ai"), 1, "{}").await;
    assert_eq!(before.unwrap().0, money("0.005"));
    assert_eq!(after.unwrap().0, money("0.01"));
}

#[tokio::test]
async fn parallel_ai_total_past_the_decimal_limit_is_an_overflow_error() {
    let source = FakeSource::new(&[]);
    source.set(
        "parallel_ai/search",
        SearchModelInfo {
            input_cost_per_query: Some(Decimal::MAX),
            tiered_pricing: None,
        },
    );
    let optional_params = r#"{"_parallel_ai_usage": [{"name": "sku_search", "count": 2}]}"#;
    let result = cost(
        &source,
        "parallel_ai/search",
        Some("parallel_ai"),
        1,
        optional_params,
    )
    .await;
    assert!(matches!(result, Err(SearchCostError::Overflow)));
}
