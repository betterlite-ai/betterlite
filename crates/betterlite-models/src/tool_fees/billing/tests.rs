#![cfg(test)]

use rstest::rstest;
use rust_decimal::Decimal;

use super::{BuiltInToolBilling, BuiltInToolParams, BuiltInToolRequest, ResolvedModel};
use crate::tool_fees::error::{LookupFailed, RateField, ToolFeeError};
use crate::tool_fees::model_info::{Rate, ToolModelInfo};
use crate::tool_fees::prices::ToolPrices;
use crate::tool_fees::scalar::VectorStoreUsage;
use crate::tool_fees::tests::{
    FakePolicy, FakeSource, PRICES, info_with, money, params_with, policy,
};
use crate::tool_signals::conversions::TokenCounts;
use crate::tool_signals::kwargs::FileSearchTool;
use crate::tool_signals::response::ToolBillingResponse;

fn search_info(medium: Rate, provider: Option<&str>) -> ToolModelInfo {
    info_with(|info| {
        info.litellm_provider = provider.map(str::to_owned);
        info.search_context.medium = medium;
    })
}

fn two_cents() -> Rate {
    Rate::Value(money(2, 2))
}

fn response(source: &str) -> ToolBillingResponse {
    serde_json::from_str(source).expect("the response fixture is valid")
}

fn web_calls(count: usize) -> String {
    let items = vec![r#"{"mapping": {"type": "web_search_call"}}"#; count].join(",");
    format!(r#"{{"responses": {{"output": [{items}]}}}}"#)
}

fn file_tool(fields: &str) -> FileSearchTool {
    serde_json::from_str(&format!(r#"{{"fields": {fields}}}"#)).expect("the tool fixture is valid")
}

const FILE_CALL: &str = r#"{"responses": {"output": [{"mapping": {"type": "file_search_call"}}]}}"#;
const QUIET: &str = r#"{"other": {}}"#;
const FILE_TYPE: &str = r#"{"type": "file_search"}"#;

fn price(
    source: &FakeSource,
    policy: &FakePolicy,
    prices: &ToolPrices,
    request: &BuiltInToolRequest<'_>,
) -> Result<Decimal, ToolFeeError> {
    let billing = BuiltInToolBilling {
        source,
        policy,
        prices,
    };
    billing.cost_for_built_in_tools(request)
}

fn request<'req>(
    provider: Option<&'req str>,
    response: &'req ToolBillingResponse,
    params: &'req BuiltInToolParams,
) -> BuiltInToolRequest<'req> {
    BuiltInToolRequest {
        model: "gpt-4o",
        custom_llm_provider: provider,
        response,
        google_maps_requests: None,
        params,
    }
}

#[test]
fn maps_cost_joins_the_web_search_branch() {
    let source =
        FakeSource::default().with_entry("gpt-4o", Some("gemini"), search_info(two_cents(), None));
    let policy = policy(None, Some(money(7, 2)));
    let reply = response(&web_calls(1));
    let params = BuiltInToolParams::default();
    let mut asked = request(Some("gemini"), &reply, &params);
    asked.google_maps_requests = Some(2);
    let fee = price(&source, &policy, &PRICES, &asked);
    assert_eq!(fee, Ok(money(9, 2)));
    assert_eq!(policy.seen(), vec!["maps:gemini:2".to_owned()]);
}

#[rstest]
#[case::zero_requests_still_call_the_policy(Some(0), 1)]
#[case::missing_requests_skip_the_policy(None, 0)]
fn maps_runs_only_for_a_nonnull_count(#[case] requests: Option<i64>, #[case] calls: usize) {
    let source =
        FakeSource::default().with_entry("gpt-4o", Some("gemini"), ToolModelInfo::default());
    let policy = policy(None, Some(money(5, 0)));
    let reply = response(QUIET);
    let params = BuiltInToolParams::default();
    let mut asked = request(Some("gemini"), &reply, &params);
    asked.google_maps_requests = requests;
    let fee = price(&source, &policy, &PRICES, &asked);
    let expected = if requests.is_some() {
        money(5, 0)
    } else {
        money(0, 0)
    };
    assert_eq!(fee, Ok(expected));
    assert_eq!(policy.seen().len(), calls);
}

#[test]
fn maps_without_a_policy_price_is_zero() {
    let source =
        FakeSource::default().with_entry("gpt-4o", Some("gemini"), ToolModelInfo::default());
    let policy = FakePolicy::default();
    let reply = response(QUIET);
    let params = BuiltInToolParams::default();
    let mut asked = request(Some("gemini"), &reply, &params);
    asked.google_maps_requests = Some(3);
    assert_eq!(price(&source, &policy, &PRICES, &asked), Ok(money(0, 0)));
    assert_eq!(policy.seen(), vec!["maps:gemini:3".to_owned()]);
}

#[test]
fn maps_needs_resolved_model_info() {
    let source = FakeSource::default();
    let policy = policy(None, Some(money(5, 0)));
    let reply = response(QUIET);
    let params = BuiltInToolParams::default();
    let mut asked = request(Some("gemini"), &reply, &params);
    asked.google_maps_requests = Some(3);
    assert_eq!(price(&source, &policy, &PRICES, &asked), Ok(money(0, 0)));
    assert!(policy.seen().is_empty());
}

#[test]
fn maps_needs_a_resolved_provider() {
    let source = FakeSource::default().with_entry("gpt-4o", None, ToolModelInfo::default());
    let policy = policy(None, Some(money(5, 0)));
    let reply = response(QUIET);
    let params = BuiltInToolParams::default();
    let mut asked = request(None, &reply, &params);
    asked.google_maps_requests = Some(3);
    assert_eq!(price(&source, &policy, &PRICES, &asked), Ok(money(0, 0)));
    assert!(policy.seen().is_empty());
}

#[test]
fn only_the_web_branch_runs_when_both_signals_exist() {
    let both = r#"{"responses": {"output": [
        {"mapping": {"type": "file_search_call"}},
        {"mapping": {"type": "web_search_call"}}
    ]}}"#;
    let source =
        FakeSource::default().with_entry("gpt-4o", Some("openai"), search_info(two_cents(), None));
    let policy = FakePolicy::default();
    let reply = response(both);
    let params = params_with(|params| {
        params.file_search = Some(file_tool(FILE_TYPE));
    });
    let asked = request(Some("openai"), &reply, &params);
    assert_eq!(price(&source, &policy, &PRICES, &asked), Ok(money(2, 2)));
}

#[test]
fn the_file_branch_runs_without_a_web_signal() {
    let source =
        FakeSource::default().with_entry("gpt-4o", Some("openai"), ToolModelInfo::default());
    let policy = policy(None, Some(money(1, 1)));
    let reply = response(FILE_CALL);
    let params = params_with(|params| {
        params.file_search = Some(file_tool(FILE_TYPE));
    });
    let mut asked = request(Some("openai"), &reply, &params);
    asked.google_maps_requests = Some(1);
    assert_eq!(price(&source, &policy, &PRICES, &asked), Ok(money(1025, 4)));
}

#[rstest]
#[case::empty_object_costs_nothing("{}", "openai", Ok(money(0, 0)))]
#[case::object_with_a_field_pays_the_call(FILE_TYPE, "openai", Ok(money(25, 4)))]
#[case::azure_storage(r#"{"storage_gb": 2, "days": 3}"#, "azure", Ok(money(6, 1)))]
#[case::azure_string_quantities(r#"{"storage_gb": "2", "days": "3"}"#, "azure", Ok(money(6, 1)))]
#[case::azure_without_quantities(FILE_TYPE, "azure", Ok(money(0, 0)))]
fn the_file_wrapper_reads_the_tool_fields(
    #[case] fields: &str,
    #[case] provider: &str,
    #[case] expected: Result<Decimal, ToolFeeError>,
) {
    let source = FakeSource::default();
    let policy = FakePolicy::default();
    let reply = response(FILE_CALL);
    let params = params_with(|params| {
        params.file_search = Some(file_tool(fields));
    });
    let asked = request(Some(provider), &reply, &params);
    assert_eq!(price(&source, &policy, &PRICES, &asked), expected);
}

#[test]
fn no_file_search_tool_costs_nothing() {
    let reply = response(FILE_CALL);
    let params = BuiltInToolParams::default();
    let asked = request(Some("openai"), &reply, &params);
    let fee = price(
        &FakeSource::default(),
        &FakePolicy::default(),
        &PRICES,
        &asked,
    );
    assert_eq!(fee, Ok(money(0, 0)));
}

#[test]
fn the_file_wrapper_looks_up_with_the_requested_provider_only() {
    let source = FakeSource::default();
    let reply = response(FILE_CALL);
    let params = params_with(|params| {
        params.file_search = Some(file_tool(FILE_TYPE));
    });
    let mut asked = request(Some("azure"), &reply, &params);
    asked.model = "azure/gpt-4o";
    let fee = price(&source, &FakePolicy::default(), &PRICES, &asked);
    assert_eq!(fee, Ok(money(0, 0)));
    assert_eq!(
        source.asked(),
        vec![("azure/gpt-4o".to_owned(), Some("azure".to_owned()))]
    );
}

#[test]
fn a_provider_price_of_zero_prevents_the_generic_fallback() {
    let source =
        FakeSource::default().with_entry("gpt-4o", Some("gemini"), search_info(two_cents(), None));
    let policy = policy(Some(money(0, 0)), None);
    let reply = response(&web_calls(3));
    let params = BuiltInToolParams::default();
    let asked = request(Some("gemini"), &reply, &params);
    assert_eq!(price(&source, &policy, &PRICES, &asked), Ok(money(0, 0)));
}

#[test]
fn a_provider_price_replaces_the_per_call_rate() {
    let source =
        FakeSource::default().with_entry("gpt-4o", Some("gemini"), search_info(two_cents(), None));
    let policy = policy(Some(money(9, 1)), None);
    let reply = response(&web_calls(3));
    let params = BuiltInToolParams::default();
    let asked = request(Some("gemini"), &reply, &params);
    assert_eq!(price(&source, &policy, &PRICES, &asked), Ok(money(9, 1)));
}

#[test]
fn no_provider_price_falls_back_to_the_per_call_rate_times_the_count() {
    let source =
        FakeSource::default().with_entry("gpt-4o", Some("gemini"), search_info(two_cents(), None));
    let policy = FakePolicy::default();
    let reply = response(&web_calls(3));
    let params = BuiltInToolParams::default();
    let asked = request(Some("gemini"), &reply, &params);
    assert_eq!(price(&source, &policy, &PRICES, &asked), Ok(money(6, 2)));
}

#[test]
fn the_policy_gets_the_repaired_usage_and_resolved_provider() {
    let source = FakeSource::default().with_entry(
        "gpt-4o",
        None,
        search_info(two_cents(), Some("anthropic")),
    );
    let policy = policy(Some(money(1, 0)), None);
    let reply = response(
        r#"{"responses": {
            "output": [{"mapping": {"type": "web_search_call"}}],
            "usage": {"server_tool_use": {"web_search_requests": 4}}
        }}"#,
    );
    let params = BuiltInToolParams::default();
    let asked = request(None, &reply, &params);
    assert_eq!(price(&source, &policy, &PRICES, &asked), Ok(money(1, 0)));
    assert_eq!(policy.seen(), vec!["web:anthropic:Some(4)".to_owned()]);
}

#[test]
fn a_response_without_usage_skips_the_provider_policy() {
    let source =
        FakeSource::default().with_entry("gpt-4o", Some("gemini"), search_info(two_cents(), None));
    let policy = policy(Some(money(1, 0)), None);
    let reply = response(&web_calls(1));
    let params = BuiltInToolParams::default();
    let asked = request(Some("gemini"), &reply, &params);
    assert_eq!(price(&source, &policy, &PRICES, &asked), Ok(money(2, 2)));
    assert!(policy.seen().is_empty());
}

#[test]
fn absent_model_info_prices_web_search_at_zero() {
    let reply = response(&web_calls(2));
    let params = BuiltInToolParams::default();
    let asked = request(Some("gemini"), &reply, &params);
    let fee = price(
        &FakeSource::default(),
        &FakePolicy::default(),
        &PRICES,
        &asked,
    );
    assert_eq!(fee, Ok(money(0, 0)));
}

#[test]
fn a_null_selected_search_rate_fails_instead_of_defaulting() {
    let source =
        FakeSource::default().with_entry("gpt-4o", Some("gemini"), search_info(Rate::Null, None));
    let reply = response(&web_calls(1));
    let params = BuiltInToolParams::default();
    let asked = request(Some("gemini"), &reply, &params);
    let fee = price(&source, &FakePolicy::default(), &PRICES, &asked);
    assert_eq!(
        fee,
        Err(ToolFeeError::NullRate {
            field: RateField::SearchContextSizeMedium
        })
    );
}

#[test]
fn a_chat_response_bills_one_search() {
    let chat =
        r#"{"chat": {"choices": [{"message": {"annotations": [{"type": "url_citation"}]}}]}}"#;
    let source =
        FakeSource::default().with_entry("gpt-4o", Some("openai"), search_info(two_cents(), None));
    let reply = response(chat);
    let params = BuiltInToolParams::default();
    let asked = request(Some("openai"), &reply, &params);
    assert_eq!(
        price(&source, &FakePolicy::default(), &PRICES, &asked),
        Ok(money(2, 2))
    );
}

#[test]
fn maps_and_branch_overflow_is_typed() {
    let source =
        FakeSource::default().with_entry("gpt-4o", Some("gemini"), search_info(two_cents(), None));
    let policy = policy(None, Some(Decimal::MAX));
    let reply = response(&web_calls(1));
    let params = BuiltInToolParams::default();
    let mut asked = request(Some("gemini"), &reply, &params);
    asked.google_maps_requests = Some(1);
    assert_eq!(
        price(&source, &policy, &PRICES, &asked),
        Err(ToolFeeError::Overflow)
    );
}

fn resolve(source: &FakeSource, model: &str, provider: Option<&str>) -> ResolvedModel {
    let policy = FakePolicy::default();
    let billing = BuiltInToolBilling {
        source,
        policy: &policy,
        prices: &PRICES,
    };
    billing.resolve_model_info(model, provider)
}

#[rstest]
#[case::requested_provider_is_kept(Some("openai"), Some("openai"), "azure")]
#[case::entry_provider_fills_a_missing_one(None, None, "azure")]
fn a_direct_hit_keeps_or_adopts_the_provider(
    #[case] requested: Option<&str>,
    #[case] key: Option<&str>,
    #[case] entry_provider: &str,
) {
    let info = search_info(Rate::Absent, Some(entry_provider));
    let source = FakeSource::default().with_entry("gpt-4o", key, info.clone());
    let resolved = resolve(&source, "gpt-4o", requested);
    let expected = requested.unwrap_or(entry_provider).to_owned();
    assert_eq!(resolved.provider, Some(expected));
    assert_eq!(resolved.info, Some(info));
    assert_eq!(source.asked().len(), 1);
}

#[test]
fn an_empty_requested_provider_adopts_the_entry_provider() {
    let info = search_info(Rate::Absent, Some("azure"));
    let source = FakeSource::default().with_entry("gpt-4o", Some(""), info);
    let resolved = resolve(&source, "gpt-4o", Some(""));
    assert_eq!(resolved.provider, Some("azure".to_owned()));
}

#[test]
fn a_slash_model_retries_without_a_provider_and_adopts_its_provider() {
    let info = search_info(Rate::Absent, Some("gemini"));
    let source = FakeSource::default().with_entry("gemini/flash", None, info.clone());
    let resolved = resolve(&source, "gemini/flash", Some("vertex_ai"));
    assert_eq!(resolved.provider, Some("gemini".to_owned()));
    assert_eq!(resolved.info, Some(info));
    assert_eq!(
        source.asked(),
        vec![
            ("gemini/flash".to_owned(), Some("vertex_ai".to_owned())),
            ("gemini/flash".to_owned(), None),
        ]
    );
}

#[rstest]
#[case::slash_model_without_a_prefix_match("gemini/flash", 2)]
#[case::plain_model_is_not_retried("gemini-flash", 1)]
fn a_failed_lookup_keeps_the_requested_provider(#[case] model: &str, #[case] lookups: usize) {
    let source = FakeSource::default();
    let resolved = resolve(&source, model, Some("vertex_ai"));
    assert_eq!(resolved.info, None);
    assert_eq!(resolved.provider, Some("vertex_ai".to_owned()));
    assert_eq!(source.asked().len(), lookups);
}

fn azure_source() -> FakeSource {
    FakeSource::default()
        .with_entry("gpt-4o", Some("azure"), ToolModelInfo::default())
        .with_container("azure", Ok(Rate::Value(money(5, 2))))
}

#[test]
fn azure_features_add_up() {
    let reply = response(QUIET);
    let params = params_with(|params| {
        params.vector_store_usage = Some(VectorStoreUsage {
            storage_gb: Some(money(2, 0)),
            days: Some(money(3, 0)),
        });
        params.computer_use_usage = Some(TokenCounts {
            input_tokens: Some(1000),
            output_tokens: None,
        });
        params.code_interpreter_sessions = Some(2);
    });
    let asked = request(Some("azure"), &reply, &params);
    let fee = price(&azure_source(), &FakePolicy::default(), &PRICES, &asked);
    assert_eq!(fee, Ok(money(370, 2)));
}

#[test]
fn azure_features_use_the_configured_prices() {
    let reply = response(QUIET);
    let params = params_with(|params| {
        params.vector_store_usage = Some(VectorStoreUsage {
            storage_gb: Some(money(2, 0)),
            days: Some(money(3, 0)),
        });
    });
    let prices = ToolPrices {
        azure_vector_per_gb_day: money(1, 0),
        ..PRICES
    };
    let asked = request(Some("azure"), &reply, &params);
    let fee = price(&azure_source(), &FakePolicy::default(), &prices, &asked);
    assert_eq!(fee, Ok(money(6, 0)));
}

#[rstest]
#[case::other_provider(Some("openai"))]
#[case::no_provider(None)]
fn only_azure_pays_assistant_features(#[case] provider: Option<&str>) {
    let reply = response(QUIET);
    let params = params_with(|params| {
        params.code_interpreter_sessions = Some(2);
        params.computer_use_usage = Some(TokenCounts {
            input_tokens: Some(1000),
            output_tokens: Some(1000),
        });
    });
    let asked = request(provider, &reply, &params);
    let fee = price(&azure_source(), &FakePolicy::default(), &PRICES, &asked);
    assert_eq!(fee, Ok(money(0, 0)));
}

#[test]
fn an_empty_vector_store_usage_is_skipped_even_with_a_null_rate() {
    let info = info_with(|info| {
        info.vector_store_per_gb_per_day = Rate::Null;
    });
    let source = FakeSource::default().with_entry("gpt-4o", Some("azure"), info);
    let reply = response(QUIET);
    let params = params_with(|params| {
        params.vector_store_usage = Some(VectorStoreUsage::default());
    });
    let asked = request(Some("azure"), &reply, &params);
    assert_eq!(
        price(&source, &FakePolicy::default(), &PRICES, &asked),
        Ok(money(0, 0))
    );
}

#[test]
fn a_half_filled_vector_store_usage_is_still_billed() {
    let info = info_with(|info| {
        info.vector_store_per_gb_per_day = Rate::Null;
    });
    let source = FakeSource::default().with_entry("gpt-4o", Some("azure"), info);
    let reply = response(QUIET);
    let params = params_with(|params| {
        params.vector_store_usage = Some(VectorStoreUsage {
            storage_gb: Some(money(2, 0)),
            days: None,
        });
    });
    let asked = request(Some("azure"), &reply, &params);
    let null_rate = ToolFeeError::NullRate {
        field: RateField::VectorStoreCostPerGbPerDay,
    };
    assert_eq!(
        price(&source, &FakePolicy::default(), &PRICES, &asked),
        Err(null_rate)
    );
}

#[test]
fn a_failed_catalog_lookup_leaves_azure_defaults() {
    let source = FakeSource::default().with_container("azure", Err(LookupFailed));
    let reply = response(QUIET);
    let params = params_with(|params| {
        params.computer_use_usage = Some(TokenCounts {
            input_tokens: Some(1000),
            output_tokens: None,
        });
        params.code_interpreter_sessions = Some(2);
    });
    let asked = request(Some("azure"), &reply, &params);
    assert_eq!(
        price(&source, &FakePolicy::default(), &PRICES, &asked),
        Ok(money(3, 0))
    );
}

#[test]
fn the_sum_of_azure_features_overflows_typed() {
    let info = info_with(|info| {
        info.code_interpreter_per_session = Rate::Value(Decimal::MAX);
        info.vector_store_per_gb_per_day = Rate::Value(Decimal::MAX);
    });
    let source = FakeSource::default().with_entry("gpt-4o", Some("azure"), info);
    let reply = response(QUIET);
    let params = params_with(|params| {
        params.vector_store_usage = Some(VectorStoreUsage {
            storage_gb: Some(money(1, 0)),
            days: Some(money(1, 0)),
        });
        params.code_interpreter_sessions = Some(1);
    });
    let asked = request(Some("azure"), &reply, &params);
    let fee = price(&source, &FakePolicy::default(), &PRICES, &asked);
    assert_eq!(fee, Err(ToolFeeError::Overflow));
}
