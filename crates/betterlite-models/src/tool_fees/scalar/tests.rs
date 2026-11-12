#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;

use super::{
    VectorStoreUsage, code_interpreter_fee, computer_use_fee, file_search_fee, vector_store_fee,
    web_search_rate,
};
use crate::tool_fees::error::{LookupFailed, RateField, ToolFeeError};
use crate::tool_fees::model_info::{Rate, SearchContextRates, ToolModelInfo};
use crate::tool_fees::prices::ToolPrices;
use crate::tool_fees::tests::{FakeSource, PRICES, info_with, money};
use crate::tool_signals::conversions::{FileSearchParams, TokenCounts};
use crate::tool_signals::kwargs::{FileSearchTool, WebSearchOptions};

fn value(units: i64, scale: u32) -> Rate {
    Rate::Value(money(units, scale))
}

fn file_tool() -> FileSearchTool {
    FileSearchTool {
        fields: indexmap::IndexMap::new(),
    }
}

fn quantities(storage: Option<Decimal>, days: Option<Decimal>) -> FileSearchParams {
    FileSearchParams {
        storage_gb: storage,
        days,
    }
}

fn two_by_three() -> FileSearchParams {
    quantities(Some(money(2, 0)), Some(money(3, 0)))
}

#[rstest]
#[case::absent_file_search_is_free(
    false,
    "openai",
    ToolModelInfo::default(),
    quantities(None, None),
    Ok(money(0, 0))
)]
#[case::empty_object_pays_the_default_call(
    true,
    "openai",
    ToolModelInfo::default(),
    quantities(None, None),
    Ok(money(25, 4))
)]
#[case::custom_per_thousand_is_returned_directly(true, "openai", info_with(|info| info.file_search_per_1k_calls = value(7, 0)), quantities(None, None), Ok(money(7, 0)))]
#[case::storage_key_is_ignored_off_azure(true, "openai", info_with(|info| {
    info.file_search_per_gb_per_day = value(5, 1);
    info.file_search_per_1k_calls = value(7, 0);
}), two_by_three(), Ok(money(7, 0)))]
#[case::azure_model_storage_rate(true, "azure", info_with(|info| info.file_search_per_gb_per_day = value(5, 1)), two_by_three(), Ok(money(3, 0)))]
#[case::azure_storage_key_blocks_the_per_call_alternative(true, "azure", info_with(|info| {
    info.file_search_per_gb_per_day = value(5, 1);
    info.file_search_per_1k_calls = value(7, 0);
}), quantities(None, None), Ok(money(0, 0)))]
#[case::azure_zero_days_is_free(true, "azure", info_with(|info| info.file_search_per_gb_per_day = value(5, 1)), quantities(Some(money(2, 0)), Some(money(0, 0))), Ok(money(0, 0)))]
#[case::azure_default_rate(
    true,
    "azure",
    ToolModelInfo::default(),
    two_by_three(),
    Ok(money(6, 1))
)]
#[case::azure_default_needs_both_quantities(
    true,
    "azure",
    ToolModelInfo::default(),
    quantities(Some(money(2, 0)), None),
    Ok(money(0, 0))
)]
#[case::azure_default_zero_storage(
    true,
    "azure",
    ToolModelInfo::default(),
    quantities(Some(money(0, 0)), Some(money(3, 0))),
    Ok(money(0, 0))
)]
#[case::negative_storage_stays_billable(true, "azure", ToolModelInfo::default(), quantities(Some(money(-2, 0)), Some(money(3, 0))), Ok(money(-6, 1)))]
#[case::null_storage_rate_with_quantities(true, "azure", info_with(|info| info.file_search_per_gb_per_day = Rate::Null), two_by_three(), Err(ToolFeeError::NullRate { field: RateField::FileSearchCostPerGbPerDay }))]
#[case::null_storage_rate_without_quantities(true, "azure", info_with(|info| info.file_search_per_gb_per_day = Rate::Null), quantities(None, None), Ok(money(0, 0)))]
#[case::null_per_thousand_rate(true, "openai", info_with(|info| info.file_search_per_1k_calls = Rate::Null), quantities(None, None), Err(ToolFeeError::NullRate { field: RateField::FileSearchCostPer1kCalls }))]
#[case::overflowing_storage(
    true,
    "azure",
    ToolModelInfo::default(),
    quantities(Some(Decimal::MAX), Some(money(2, 0))),
    Err(ToolFeeError::Overflow)
)]
fn file_search_fee_follows_key_presence_and_truthiness(
    #[case] present: bool,
    #[case] provider: &str,
    #[case] info: ToolModelInfo,
    #[case] params: FileSearchParams,
    #[case] expected: Result<Decimal, ToolFeeError>,
) {
    let tool = file_tool();
    let selected = present.then_some(&tool);
    let fee = file_search_fee(selected, Some(provider), Some(&info), params, &PRICES);
    assert_eq!(fee, expected);
}

#[rstest]
#[case::no_model_info_off_azure(None, "openai", Ok(money(25, 4)))]
#[case::no_model_info_on_azure(None, "azure", Ok(money(6, 1)))]
fn file_search_fee_without_model_info(
    #[case] info: Option<ToolModelInfo>,
    #[case] provider: &str,
    #[case] expected: Result<Decimal, ToolFeeError>,
) {
    let tool = file_tool();
    let fee = file_search_fee(
        Some(&tool),
        Some(provider),
        info.as_ref(),
        two_by_three(),
        &PRICES,
    );
    assert_eq!(fee, expected);
}

#[test]
fn file_search_fee_uses_the_supplied_prices() {
    let prices = ToolPrices {
        openai_search_per_1k: money(9, 0),
        azure_search_per_gb_day: money(2, 0),
        ..PRICES
    };
    let tool = file_tool();
    let call = file_search_fee(Some(&tool), Some("openai"), None, two_by_three(), &prices);
    let storage = file_search_fee(Some(&tool), Some("azure"), None, two_by_three(), &prices);
    assert_eq!(call, Ok(money(9, 0)));
    assert_eq!(storage, Ok(money(12, 0)));
}

#[rstest]
#[case::default_azure_rate("azure", ToolModelInfo::default(), money(6, 1))]
#[case::model_rate_beats_the_default("azure", info_with(|info| info.vector_store_per_gb_per_day = value(2, 1)), money(12, 1))]
#[case::model_rate_applies_off_azure("openai", info_with(|info| info.vector_store_per_gb_per_day = value(2, 1)), money(12, 1))]
#[case::zero_model_rate_is_kept("azure", info_with(|info| info.vector_store_per_gb_per_day = value(0, 0)), money(0, 0))]
#[case::other_providers_are_free("openai", ToolModelInfo::default(), money(0, 0))]
fn vector_store_fee_selects_its_rate(
    #[case] provider: &str,
    #[case] info: ToolModelInfo,
    #[case] expected: Decimal,
) {
    let usage = VectorStoreUsage {
        storage_gb: Some(money(2, 0)),
        days: Some(money(3, 0)),
    };
    let fee = vector_store_fee(usage, Some(provider), Some(&info), &PRICES);
    assert_eq!(fee, Ok(expected));
}

#[test]
fn vector_store_fee_edges() {
    let mut usage = VectorStoreUsage {
        storage_gb: Some(money(-2, 0)),
        days: Some(money(3, 0)),
    };
    let none = ToolModelInfo::default();
    assert_eq!(
        vector_store_fee(usage, Some("azure"), Some(&none), &PRICES),
        Ok(money(-6, 1))
    );
    usage.days = None;
    assert_eq!(
        vector_store_fee(usage, Some("azure"), Some(&none), &PRICES),
        Ok(money(0, 0))
    );
    let null = info_with(|info| info.vector_store_per_gb_per_day = Rate::Null);
    assert_eq!(
        vector_store_fee(usage, Some("azure"), Some(&null), &PRICES),
        Err(ToolFeeError::NullRate {
            field: RateField::VectorStoreCostPerGbPerDay
        })
    );
    let huge = VectorStoreUsage {
        storage_gb: Some(Decimal::MAX),
        days: Some(money(2, 0)),
    };
    assert_eq!(
        vector_store_fee(huge, Some("azure"), None, &PRICES),
        Err(ToolFeeError::Overflow)
    );
}

fn tokens(input: Option<i64>, output: Option<i64>) -> TokenCounts {
    TokenCounts {
        input_tokens: input,
        output_tokens: output,
    }
}

fn computer_rates(input: Rate, output: Rate) -> ToolModelInfo {
    info_with(|info| {
        info.computer_input_per_1k = input;
        info.computer_output_per_1k = output;
    })
}

#[rstest]
#[case::input_default_is_three_per_thousand(
    "azure",
    ToolModelInfo::default(),
    tokens(Some(1000), None),
    Ok(money(3, 0))
)]
#[case::output_default_is_twelve_per_thousand(
    "azure",
    ToolModelInfo::default(),
    tokens(None, Some(1000)),
    Ok(money(12, 0))
)]
#[case::both_default_sides(
    "azure",
    ToolModelInfo::default(),
    tokens(Some(500), Some(250)),
    Ok(money(45, 1))
)]
#[case::other_providers_are_free(
    "openai",
    ToolModelInfo::default(),
    tokens(Some(1000), Some(1000)),
    Ok(money(0, 0))
)]
#[case::zero_tokens_are_free(
    "azure",
    ToolModelInfo::default(),
    tokens(Some(0), Some(0)),
    Ok(money(0, 0))
)]
#[case::missing_tokens_are_free(
    "azure",
    ToolModelInfo::default(),
    tokens(None, None),
    Ok(money(0, 0))
)]
#[case::custom_rates_on_both_sides(
    "azure",
    computer_rates(value(1, 0), value(2, 0)),
    tokens(Some(2000), Some(1000)),
    Ok(money(4, 0))
)]
#[case::one_custom_side_keeps_the_other_free(
    "azure",
    computer_rates(value(1, 0), Rate::Absent),
    tokens(Some(1000), Some(1000)),
    Ok(money(1, 0))
)]
#[case::custom_output_only(
    "azure",
    computer_rates(Rate::Absent, value(2, 0)),
    tokens(Some(1000), Some(1000)),
    Ok(money(2, 0))
)]
#[case::two_zero_rates_use_the_defaults(
    "azure",
    computer_rates(value(0, 0), value(0, 0)),
    tokens(Some(1000), Some(1000)),
    Ok(money(15, 0))
)]
#[case::null_rates_are_not_truthy(
    "azure",
    computer_rates(Rate::Null, Rate::Null),
    tokens(Some(1000), Some(1000)),
    Ok(money(15, 0))
)]
#[case::negative_tokens_stay_billable("azure", ToolModelInfo::default(), tokens(Some(-1000), None), Ok(money(-3, 0)))]
#[case::null_side_with_tokens_fails("azure", computer_rates(Rate::Null, value(2, 0)), tokens(Some(1000), None), Err(ToolFeeError::NullRate { field: RateField::ComputerInputPer1k }))]
#[case::null_side_without_tokens_is_unread(
    "azure",
    computer_rates(Rate::Null, value(2, 0)),
    tokens(None, Some(1000)),
    Ok(money(2, 0))
)]
fn computer_use_fee_selects_custom_or_default_rates(
    #[case] provider: &str,
    #[case] info: ToolModelInfo,
    #[case] counts: TokenCounts,
    #[case] expected: Result<Decimal, ToolFeeError>,
) {
    let fee = computer_use_fee(counts, Some(provider), Some(&info), &PRICES);
    assert_eq!(fee, expected);
}

#[test]
fn computer_use_fee_edges() {
    let prices = ToolPrices {
        azure_computer_input_per_1k: money(1, 0),
        azure_computer_output_per_1k: money(2, 0),
        ..PRICES
    };
    let counts = tokens(Some(1000), Some(1000));
    assert_eq!(
        computer_use_fee(counts, Some("azure"), None, &prices),
        Ok(money(3, 0))
    );
    assert_eq!(
        computer_use_fee(counts, None, None, &PRICES),
        Ok(money(0, 0))
    );
    let huge = tokens(Some(i64::MAX), None);
    let rich = computer_rates(Rate::Value(Decimal::MAX), Rate::Absent);
    assert_eq!(
        computer_use_fee(huge, Some("azure"), Some(&rich), &PRICES),
        Err(ToolFeeError::Overflow)
    );
}

fn session_rate(rate: Rate) -> ToolModelInfo {
    info_with(|info| info.code_interpreter_per_session = rate)
}

#[rstest]
#[case::missing_sessions(None, "azure", ToolModelInfo::default(), Ok(money(0, 0)))]
#[case::zero_sessions(Some(0), "azure", session_rate(value(3, 2)), Ok(money(0, 0)))]
#[case::model_rate(Some(3), "azure", session_rate(value(3, 2)), Ok(money(9, 2)))]
#[case::free_container_rate(Some(3), "azure", session_rate(value(0, 0)), Ok(money(0, 0)))]
#[case::negative_sessions_stay_billable(Some(-2), "azure", session_rate(value(3, 2)), Ok(money(-6, 2)))]
#[case::model_rate_applies_to_any_provider(
    Some(1),
    "openai",
    session_rate(value(3, 2)),
    Ok(money(3, 2))
)]
#[case::null_model_rate(Some(1), "azure", session_rate(Rate::Null), Err(ToolFeeError::NullRate { field: RateField::CodeInterpreterCostPerSession }))]
#[case::container_rate_from_the_catalog(
    Some(2),
    "azure",
    ToolModelInfo::default(),
    Ok(money(1, 1))
)]
#[case::catalog_rate_for_any_provider(Some(2), "openai", ToolModelInfo::default(), Ok(money(1, 1)))]
fn code_interpreter_fee_prefers_the_model_key(
    #[case] sessions: Option<i64>,
    #[case] provider: &str,
    #[case] info: ToolModelInfo,
    #[case] expected: Result<Decimal, ToolFeeError>,
) {
    let source = FakeSource::default()
        .with_container("azure", Ok(value(5, 2)))
        .with_container("openai", Ok(value(5, 2)));
    let fee = code_interpreter_fee(sessions, Some(provider), Some(&info), &source);
    assert_eq!(fee, expected);
}

#[rstest]
#[case::zero_catalog_price_is_kept(Ok(value(0, 0)))]
#[case::absent_catalog_price(Ok(Rate::Absent))]
#[case::null_catalog_price(Ok(Rate::Null))]
#[case::failed_lookup(Err(LookupFailed))]
fn code_interpreter_fee_is_free_without_a_container_price(
    #[case] container: Result<Rate, LookupFailed>,
) {
    let source = FakeSource::default().with_container("azure", container);
    let fee = code_interpreter_fee(Some(4), Some("azure"), None, &source);
    assert_eq!(fee, Ok(money(0, 0)));
    assert_eq!(source.asked(), vec![("azure/container".to_owned(), None)]);
}

#[rstest]
#[case::no_provider(None)]
#[case::empty_provider(Some(""))]
fn code_interpreter_fee_skips_the_catalog_without_a_provider(#[case] provider: Option<&str>) {
    let source = FakeSource::default().with_container("", Ok(value(5, 2)));
    let fee = code_interpreter_fee(Some(2), provider, None, &source);
    assert_eq!(fee, Ok(money(0, 0)));
    assert!(source.asked().is_empty());
}

#[test]
fn code_interpreter_fee_overflow_is_typed() {
    let source = FakeSource::default();
    let info = session_rate(Rate::Value(Decimal::MAX));
    assert_eq!(
        code_interpreter_fee(Some(2), Some("azure"), Some(&info), &source),
        Err(ToolFeeError::Overflow)
    );
}

fn search_rates(low: Rate, medium: Rate, high: Rate) -> ToolModelInfo {
    info_with(|info| info.search_context = SearchContextRates { low, medium, high })
}

fn size_option(size: &str) -> WebSearchOptions {
    serde_json::from_str(&format!(
        r#"{{"fields": {{"search_context_size": {size}}}}}"#
    ))
    .expect("the options fixture is valid")
}

#[rstest]
#[case::low(r#""low""#, money(1, 2))]
#[case::medium(r#""medium""#, money(2, 2))]
#[case::high(r#""high""#, money(3, 2))]
#[case::unknown_size_uses_medium(r#""huge""#, money(2, 2))]
#[case::wrong_case_uses_medium(r#""LOW""#, money(2, 2))]
#[case::null_size_uses_medium("null", money(2, 2))]
#[case::non_text_size_uses_medium("1", money(2, 2))]
fn web_search_rate_selects_the_exact_size_key(#[case] size: &str, #[case] expected: Decimal) {
    let info = search_rates(value(1, 2), value(2, 2), value(3, 2));
    let options = size_option(size);
    assert_eq!(web_search_rate(Some(&options), Some(&info)), Ok(expected));
}

#[test]
fn web_search_rate_without_options_or_info() {
    let info = search_rates(value(1, 2), value(2, 2), value(3, 2));
    assert_eq!(web_search_rate(None, Some(&info)), Ok(money(2, 2)));
    assert_eq!(web_search_rate(None, None), Ok(money(0, 0)));
    let empty = ToolModelInfo::default();
    assert_eq!(web_search_rate(None, Some(&empty)), Ok(money(0, 0)));
}

#[rstest]
#[case::low(r#""low""#, RateField::SearchContextSizeLow)]
#[case::medium(r#""medium""#, RateField::SearchContextSizeMedium)]
#[case::high(r#""high""#, RateField::SearchContextSizeHigh)]
fn a_null_selected_search_rate_is_not_the_default(#[case] size: &str, #[case] field: RateField) {
    let info = search_rates(Rate::Null, Rate::Null, Rate::Null);
    let options = size_option(size);
    assert_eq!(
        web_search_rate(Some(&options), Some(&info)),
        Err(ToolFeeError::NullRate { field })
    );
}

#[test]
fn a_null_rate_that_is_not_selected_is_harmless() {
    let info = search_rates(Rate::Null, value(2, 2), Rate::Null);
    let options = size_option(r#""medium""#);
    assert_eq!(
        web_search_rate(Some(&options), Some(&info)),
        Ok(money(2, 2))
    );
}
