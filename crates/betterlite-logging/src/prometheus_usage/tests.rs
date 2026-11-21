#![cfg(test)]

use std::time::Instant;

use betterlite_models::decisions::Json;
use betterlite_prometheus::collector_factory::collector::MetricValue;
use rust_decimal::Decimal;

use crate::end_user::EndUserCostTrackingSettings;
use crate::prometheus_helpers::bounded_series_tracker::SeriesLimits;

use self::support::{
    DecimalFromMetric as _, FakeCounters, Fixture, MCP_LABELS, REQUEST_LABELS, Recorded,
    ZERO_COST_LABELS, diagnostic, emit_requests, emit_zero_cost, finite, increments, mcp_payload,
    number, object, one, payload, with_usage,
};
use super::cache::increment_cache_metrics;
use super::capture_rate::set_spend_capture_rate;
use super::error::PrometheusUsageError;
use super::mcp::increment_mcp_tool_calls;
use super::media::increment_media_metrics;
use super::metric::UsageMetric;
use super::request_spend::{increment_request_and_spend, increment_zero_cost_requests};
use super::tokens::{increment_token_detail_metrics, increment_token_metrics};
use super::usage_object::{resolve_cache_write_tokens, resolve_provider_cache_tokens};

mod support;

#[test]
fn request_spend_increments_requests_then_spend() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&REQUEST_LABELS);
    increment_request_and_spend(
        &counters,
        &payload(finite("0.25").decimal()),
        fixture.labels(),
        Instant::now(),
    )
    .unwrap();
    assert_eq!(
        increments(&counters),
        [
            (UsageMetric::Requests, one()),
            (UsageMetric::Spend, finite("0.25")),
        ]
    );
    let events = counters.events();
    let Recorded::Increment { labels, .. } = &events[0] else {
        panic!("the request counter did not increment");
    };
    assert_eq!(labels.get("model").map(String::as_str), Some("gpt-4o"));
    assert_eq!(labels.get("end_user").map(String::as_str), Some("alice"));
}

#[test]
fn request_spend_negative_cost_errors_and_stops() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&REQUEST_LABELS);
    let error = increment_request_and_spend(
        &counters,
        &payload(finite("-0.5").decimal()),
        fixture.labels(),
        Instant::now(),
    )
    .unwrap_err();
    assert!(matches!(error, PrometheusUsageError::NegativeAmount { .. }));
    assert_eq!(increments(&counters), [(UsageMetric::Requests, one())]);
}

#[test]
fn request_spend_counter_runs_end_user_cap() {
    let counters = FakeCounters::with_labels(&REQUEST_LABELS).with_limits(SeriesLimits {
        max_series: Some(1),
        ttl: None,
        cleanup_interval: None,
    });
    emit_requests(&counters, &["a", "b"]);
    assert!(
        counters
            .events()
            .iter()
            .any(|event| matches!(event, Recorded::Removal { .. }))
    );
}

#[test]
fn zero_cost_counter_carries_reason_label() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&ZERO_COST_LABELS);
    increment_zero_cost_requests(&counters, Some(&diagnostic()), fixture.labels()).unwrap();
    assert_eq!(
        increments(&counters),
        [(UsageMetric::ZeroCostRequests, one())]
    );
    let events = counters.events();
    let Recorded::Increment { labels, .. } = &events[0] else {
        panic!("the zero cost counter did not increment");
    };
    assert_eq!(
        labels.get("reason").map(String::as_str),
        Some("missing_pricing_key")
    );
    assert_eq!(labels.get("model").map(String::as_str), Some("gpt-4o"));
}

#[test]
fn zero_cost_reason_omitted_when_unsupported() {
    let fixture = Fixture::with_end_user("alice");
    let counters =
        FakeCounters::with_labels(&[("litellm_zero_cost_requests_total", &["model", "end_user"])]);
    increment_zero_cost_requests(&counters, Some(&diagnostic()), fixture.labels()).unwrap();
    let events = counters.events();
    let Recorded::Increment { labels, .. } = &events[0] else {
        panic!("the zero cost counter did not increment");
    };
    assert!(!labels.contains_key("reason"));
}

#[test]
fn zero_cost_absent_diagnostic_increments_nothing() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&ZERO_COST_LABELS);
    increment_zero_cost_requests(&counters, None, fixture.labels()).unwrap();
    assert!(counters.events().is_empty());
}

#[test]
fn zero_cost_counter_bypasses_end_user_cap() {
    let counters = FakeCounters::with_labels(&ZERO_COST_LABELS).with_limits(SeriesLimits {
        max_series: Some(1),
        ttl: None,
        cleanup_interval: None,
    });
    emit_zero_cost(&counters, &["a", "b", "c"]);
    assert_eq!(counters.events().len(), 3);
    assert!(
        counters
            .events()
            .iter()
            .all(|event| matches!(event, Recorded::Increment { .. }))
    );
}

#[test]
fn token_counters_increment_in_order_even_at_zero() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    increment_token_metrics(
        &counters,
        &payload(Decimal::ZERO),
        fixture.labels(),
        Instant::now(),
    )
    .unwrap();
    assert_eq!(
        increments(&counters),
        [
            (UsageMetric::TotalTokens, finite("0")),
            (UsageMetric::InputTokens, finite("0")),
            (UsageMetric::OutputTokens, finite("0")),
        ]
    );
}

#[test]
fn token_counters_use_payload_counts() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    let mut payload = payload(Decimal::ZERO);
    payload.total_tokens = 7;
    payload.prompt_tokens = 3;
    payload.completion_tokens = 4;
    increment_token_metrics(&counters, &payload, fixture.labels(), Instant::now()).unwrap();
    assert_eq!(
        increments(&counters),
        [
            (UsageMetric::TotalTokens, finite("7")),
            (UsageMetric::InputTokens, finite("3")),
            (UsageMetric::OutputTokens, finite("4")),
        ]
    );
}

#[test]
fn token_detail_counters_increment_sparse_values_in_order() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    let usage = object(vec![
        (
            "prompt_tokens_details",
            object(vec![
                ("cached_tokens", number("5")),
                ("cache_creation_tokens", number("4")),
                ("audio_tokens", number("2")),
            ]),
        ),
        (
            "completion_tokens_details",
            object(vec![
                ("reasoning_tokens", number("3")),
                ("audio_tokens", number("1")),
            ]),
        ),
    ]);
    increment_token_detail_metrics(
        &counters,
        &with_usage(payload(Decimal::ZERO), usage),
        fixture.labels(),
        Instant::now(),
    )
    .unwrap();
    assert_eq!(
        increments(&counters),
        [
            (UsageMetric::InputCachedTokens, finite("5")),
            (UsageMetric::InputCacheCreationTokens, finite("4")),
            (UsageMetric::InputAudioTokens, finite("2")),
            (UsageMetric::OutputReasoningTokens, finite("3")),
            (UsageMetric::OutputAudioTokens, finite("1")),
        ]
    );
}

#[test]
fn token_detail_counters_keep_exact_sparse_decimals() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    let usage = object(vec![(
        "prompt_tokens_details",
        object(vec![("cached_tokens", number("9007199254740993.5"))]),
    )]);
    increment_token_detail_metrics(
        &counters,
        &with_usage(payload(Decimal::ZERO), usage),
        fixture.labels(),
        Instant::now(),
    )
    .unwrap();
    assert_eq!(
        increments(&counters),
        [(UsageMetric::InputCachedTokens, finite("9007199254740993.5"))]
    );
}

#[test]
fn token_detail_counters_skip_non_positive_and_non_numeric() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    let usage = object(vec![
        (
            "prompt_tokens_details",
            object(vec![
                ("cached_tokens", number("0")),
                ("cache_creation_tokens", number("-2")),
                ("audio_tokens", Json::Bool(true)),
            ]),
        ),
        (
            "completion_tokens_details",
            object(vec![
                ("reasoning_tokens", Json::Text("3".to_owned())),
                ("audio_tokens", number("1.5")),
            ]),
        ),
    ]);
    increment_token_detail_metrics(
        &counters,
        &with_usage(payload(Decimal::ZERO), usage),
        fixture.labels(),
        Instant::now(),
    )
    .unwrap();
    assert_eq!(
        increments(&counters),
        [(UsageMetric::OutputAudioTokens, finite("1.5"))]
    );
}

#[test]
fn token_detail_counters_skip_without_usage_object() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    increment_token_detail_metrics(
        &counters,
        &payload(Decimal::ZERO),
        fixture.labels(),
        Instant::now(),
    )
    .unwrap();
    assert!(counters.events().is_empty());
}

#[test]
fn token_detail_counters_skip_non_object_usage() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    increment_token_detail_metrics(
        &counters,
        &with_usage(payload(Decimal::ZERO), Json::Text("usage".to_owned())),
        fixture.labels(),
        Instant::now(),
    )
    .unwrap();
    assert!(counters.events().is_empty());
}

#[test]
fn token_detail_counters_read_details_only_from_objects() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    let usage = object(vec![
        ("prompt_tokens_details", Json::List(Vec::new())),
        (
            "completion_tokens_details",
            Json::Text("details".to_owned()),
        ),
    ]);
    increment_token_detail_metrics(
        &counters,
        &with_usage(payload(Decimal::ZERO), usage),
        fixture.labels(),
        Instant::now(),
    )
    .unwrap();
    assert!(counters.events().is_empty());
}

#[test]
fn token_detail_counters_cache_creation_falls_back() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    let usage = object(vec![(
        "prompt_tokens_details",
        object(vec![
            ("cache_write_tokens", Json::Text("none".to_owned())),
            ("cache_creation_tokens", number("3")),
        ]),
    )]);
    increment_token_detail_metrics(
        &counters,
        &with_usage(payload(Decimal::ZERO), usage),
        fixture.labels(),
        Instant::now(),
    )
    .unwrap();
    assert_eq!(
        increments(&counters),
        [(UsageMetric::InputCacheCreationTokens, finite("3"))]
    );
}

#[test]
fn media_counters_increment_duration_and_images() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    let usage = object(vec![
        ("duration_seconds", number("2.5")),
        ("output_image_count", number("2")),
    ]);
    increment_media_metrics(
        &counters,
        &with_usage(payload(Decimal::ZERO), usage),
        fixture.labels(),
        Instant::now(),
    )
    .unwrap();
    assert_eq!(
        increments(&counters),
        [
            (UsageMetric::VideoDurationSeconds, finite("2.5")),
            (UsageMetric::ImagesGenerated, finite("2")),
        ]
    );
}

#[test]
fn media_counters_skip_non_positive_and_non_numeric() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    let usage = object(vec![
        ("duration_seconds", number("0")),
        ("output_image_count", Json::Text("3".to_owned())),
    ]);
    increment_media_metrics(
        &counters,
        &with_usage(payload(Decimal::ZERO), usage),
        fixture.labels(),
        Instant::now(),
    )
    .unwrap();
    assert!(counters.events().is_empty());
}

#[test]
fn media_counters_emit_infinity_above_decimal_range() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    let usage = object(vec![("duration_seconds", number("1e40"))]);
    increment_media_metrics(
        &counters,
        &with_usage(payload(Decimal::ZERO), usage),
        fixture.labels(),
        Instant::now(),
    )
    .unwrap();
    assert_eq!(
        increments(&counters),
        [(
            UsageMetric::VideoDurationSeconds,
            MetricValue::PositiveInfinity
        )]
    );
}

#[test]
fn media_counters_skip_number_underflowing_to_zero() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    let usage = object(vec![
        ("duration_seconds", number("1e-999999")),
        ("output_image_count", number("1e40")),
    ]);
    increment_media_metrics(
        &counters,
        &with_usage(payload(Decimal::ZERO), usage),
        fixture.labels(),
        Instant::now(),
    )
    .unwrap();
    assert_eq!(
        increments(&counters),
        [(UsageMetric::ImagesGenerated, MetricValue::PositiveInfinity)]
    );
}

#[test]
fn token_detail_counters_skip_zero_cache_creation_tokens() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    let usage = object(vec![(
        "prompt_tokens_details",
        object(vec![("cache_creation_tokens", number("0"))]),
    )]);
    increment_token_detail_metrics(
        &counters,
        &with_usage(payload(Decimal::ZERO), usage),
        fixture.labels(),
        Instant::now(),
    )
    .unwrap();
    assert!(counters.events().is_empty());
}

#[test]
fn media_counters_skip_without_usage_object() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    increment_media_metrics(
        &counters,
        &payload(Decimal::ZERO),
        fixture.labels(),
        Instant::now(),
    )
    .unwrap();
    assert!(counters.events().is_empty());
}

#[test]
fn cache_hit_true_increments_hits_then_cached_tokens() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    let mut payload = payload(Decimal::ZERO);
    payload.cache_hit = Some(true);
    payload.total_tokens = 5;
    increment_cache_metrics(&counters, &payload, fixture.labels(), Instant::now()).unwrap();
    assert_eq!(
        increments(&counters),
        [
            (UsageMetric::CacheHits, one()),
            (UsageMetric::CachedTokens, finite("5")),
        ]
    );
}

#[test]
fn cache_hit_true_with_zero_tokens_increments_only_hits() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    let mut payload = payload(Decimal::ZERO);
    payload.cache_hit = Some(true);
    increment_cache_metrics(&counters, &payload, fixture.labels(), Instant::now()).unwrap();
    assert_eq!(increments(&counters), [(UsageMetric::CacheHits, one())]);
}

#[test]
fn cache_hit_false_increments_misses() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    let mut payload = payload(Decimal::ZERO);
    payload.cache_hit = Some(false);
    increment_cache_metrics(&counters, &payload, fixture.labels(), Instant::now()).unwrap();
    assert_eq!(increments(&counters), [(UsageMetric::CacheMisses, one())]);
}

#[test]
fn cache_hit_absent_increments_no_cache_counters() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    increment_cache_metrics(
        &counters,
        &payload(Decimal::ZERO),
        fixture.labels(),
        Instant::now(),
    )
    .unwrap();
    assert!(counters.events().is_empty());
}

#[test]
fn cache_provider_counters_run_without_cache_hit() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    let usage = object(vec![
        ("cache_read_input_tokens", number("4")),
        ("cache_creation_input_tokens", number("2")),
    ]);
    increment_cache_metrics(
        &counters,
        &with_usage(payload(Decimal::ZERO), usage),
        fixture.labels(),
        Instant::now(),
    )
    .unwrap();
    assert_eq!(
        increments(&counters),
        [
            (UsageMetric::ProviderCacheReadTokens, finite("4")),
            (UsageMetric::ProviderCacheCreationTokens, finite("2")),
        ]
    );
}

#[test]
fn cache_write_tokens_first_integer_wins() {
    let details = object(vec![
        ("cache_write_tokens", number("3")),
        ("cache_creation_tokens", number("4")),
    ]);
    assert_eq!(resolve_cache_write_tokens(Some(&details)), Some(3));
}

#[test]
fn cache_write_tokens_skips_non_integer_and_bool() {
    let skipped = object(vec![
        ("cache_write_tokens", Json::Bool(true)),
        ("cache_creation_tokens", number("5")),
    ]);
    assert_eq!(resolve_cache_write_tokens(Some(&skipped)), Some(5));
    let text = object(vec![("cache_write_tokens", Json::Text("3".to_owned()))]);
    assert_eq!(resolve_cache_write_tokens(Some(&text)), None);
}

#[test]
fn cache_write_tokens_none_for_non_object() {
    let text = Json::Text("details".to_owned());
    assert_eq!(resolve_cache_write_tokens(Some(&text)), None);
}

#[test]
fn cache_write_tokens_none_without_integers() {
    assert_eq!(resolve_cache_write_tokens(Some(&object(Vec::new()))), None);
}

#[test]
fn provider_cache_explicit_fields_win() {
    let usage = object(vec![
        ("cache_read_input_tokens", number("0")),
        ("cache_creation_input_tokens", number("3")),
        (
            "prompt_tokens_details",
            object(vec![
                ("cached_tokens", number("9")),
                ("cache_write_tokens", number("7")),
            ]),
        ),
    ]);
    let Json::Object(fields) = usage else {
        panic!("the fixture is not an object");
    };
    assert_eq!(resolve_provider_cache_tokens(&fields), (0, 3));
}

#[test]
fn provider_cache_falls_back_when_fields_absent() {
    let usage = object(vec![(
        "prompt_tokens_details",
        object(vec![
            ("cached_tokens", number("4")),
            ("cache_write_tokens", number("2")),
        ]),
    )]);
    let Json::Object(fields) = usage else {
        panic!("the fixture is not an object");
    };
    assert_eq!(resolve_provider_cache_tokens(&fields), (4, 2));
}

#[test]
fn provider_cache_falls_back_when_fields_null() {
    let usage = object(vec![
        ("cache_read_input_tokens", Json::Null),
        ("cache_creation_input_tokens", Json::Null),
        (
            "prompt_tokens_details",
            object(vec![
                ("cached_tokens", number("4")),
                ("cache_creation_tokens", number("2")),
            ]),
        ),
    ]);
    let Json::Object(fields) = usage else {
        panic!("the fixture is not an object");
    };
    assert_eq!(resolve_provider_cache_tokens(&fields), (4, 2));
}

#[test]
fn provider_cache_wrong_types_do_not_fall_back() {
    let usage = object(vec![
        ("cache_read_input_tokens", Json::Text("5".to_owned())),
        (
            "prompt_tokens_details",
            object(vec![("cached_tokens", number("9"))]),
        ),
    ]);
    let Json::Object(fields) = usage else {
        panic!("the fixture is not an object");
    };
    assert_eq!(resolve_provider_cache_tokens(&fields), (0, 0));
}

#[test]
fn provider_cache_bool_is_not_an_integer() {
    let usage = object(vec![
        ("cache_read_input_tokens", Json::Bool(true)),
        ("cache_creation_input_tokens", Json::Bool(true)),
        (
            "prompt_tokens_details",
            object(vec![("cached_tokens", number("9"))]),
        ),
    ]);
    let Json::Object(fields) = usage else {
        panic!("the fixture is not an object");
    };
    assert_eq!(resolve_provider_cache_tokens(&fields), (0, 0));
}

#[test]
fn provider_cache_integer_outside_i64_is_absent() {
    let usage = object(vec![(
        "cache_read_input_tokens",
        number("9223372036854775808"),
    )]);
    let Json::Object(fields) = usage else {
        panic!("the fixture is not an object");
    };
    assert_eq!(resolve_provider_cache_tokens(&fields), (0, 0));
}

#[test]
fn provider_cache_skips_zero_and_negative_counts() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    let usage = object(vec![
        ("cache_read_input_tokens", number("0")),
        ("cache_creation_input_tokens", number("-2")),
    ]);
    increment_cache_metrics(
        &counters,
        &with_usage(payload(Decimal::ZERO), usage),
        fixture.labels(),
        Instant::now(),
    )
    .unwrap();
    assert!(counters.events().is_empty());
}

#[test]
fn provider_cache_skips_zero_creation_tokens() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    let usage = object(vec![
        ("cache_read_input_tokens", number("3")),
        ("cache_creation_input_tokens", number("0")),
    ]);
    increment_cache_metrics(
        &counters,
        &with_usage(payload(Decimal::ZERO), usage),
        fixture.labels(),
        Instant::now(),
    )
    .unwrap();
    assert_eq!(
        increments(&counters),
        [(UsageMetric::ProviderCacheReadTokens, finite("3"))]
    );
}

#[test]
fn mcp_tool_calls_use_reduced_label_values() {
    let fixture = Fixture::new(&[
        ("model", "gpt-4o"),
        ("team", "team-1"),
        ("end_user", "alice"),
    ]);
    let counters = FakeCounters::with_labels(&MCP_LABELS);
    let metadata = object(vec![
        ("name", Json::Text("search".to_owned())),
        ("mcp_server_name", Json::Text("server".to_owned())),
    ]);
    increment_mcp_tool_calls(
        &counters,
        &mcp_payload(Some(metadata)),
        fixture.labels(),
        Decimal::ZERO,
        Instant::now(),
    )
    .unwrap();
    let events = counters.events();
    let Recorded::Increment { metric, labels, .. } = &events[0] else {
        panic!("the MCP tool call counter did not increment");
    };
    assert_eq!(*metric, UsageMetric::McpToolCalls);
    assert_eq!(labels.get("model").map(String::as_str), Some("None"));
    assert_eq!(labels.get("team").map(String::as_str), Some("team-1"));
    assert_eq!(labels.get("end_user").map(String::as_str), Some("alice"));
    assert_eq!(
        labels.get("mcp_tool_name").map(String::as_str),
        Some("search")
    );
    assert_eq!(
        labels.get("mcp_server_name").map(String::as_str),
        Some("server")
    );
}

#[test]
fn mcp_tool_calls_non_string_name_renders_none() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&MCP_LABELS);
    let metadata = object(vec![("name", number("5")), ("mcp_server_name", Json::Null)]);
    increment_mcp_tool_calls(
        &counters,
        &mcp_payload(Some(metadata)),
        fixture.labels(),
        Decimal::ZERO,
        Instant::now(),
    )
    .unwrap();
    let events = counters.events();
    let Recorded::Increment { labels, .. } = &events[0] else {
        panic!("the MCP tool call counter did not increment");
    };
    assert_eq!(
        labels.get("mcp_tool_name").map(String::as_str),
        Some("None")
    );
    assert_eq!(
        labels.get("mcp_server_name").map(String::as_str),
        Some("None")
    );
}

#[test]
fn mcp_tool_calls_spend_only_when_positive() {
    let fixture = Fixture::with_end_user("alice");
    let metadata = object(vec![("name", Json::Text("search".to_owned()))]);
    let free = FakeCounters::with_labels(&MCP_LABELS);
    increment_mcp_tool_calls(
        &free,
        &mcp_payload(Some(metadata.clone())),
        fixture.labels(),
        Decimal::ZERO,
        Instant::now(),
    )
    .unwrap();
    assert_eq!(increments(&free), [(UsageMetric::McpToolCalls, one())]);
    let paid = FakeCounters::with_labels(&MCP_LABELS);
    increment_mcp_tool_calls(
        &paid,
        &mcp_payload(Some(metadata)),
        fixture.labels(),
        finite("0.5").decimal(),
        Instant::now(),
    )
    .unwrap();
    assert_eq!(
        increments(&paid),
        [
            (UsageMetric::McpToolCalls, one()),
            (UsageMetric::McpToolCallSpend, finite("0.5")),
        ]
    );
}

#[test]
fn mcp_tool_calls_skip_non_object_metadata() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&MCP_LABELS);
    increment_mcp_tool_calls(
        &counters,
        &mcp_payload(None),
        fixture.labels(),
        Decimal::ONE,
        Instant::now(),
    )
    .unwrap();
    increment_mcp_tool_calls(
        &counters,
        &mcp_payload(Some(Json::Text("metadata".to_owned()))),
        fixture.labels(),
        Decimal::ONE,
        Instant::now(),
    )
    .unwrap();
    assert!(counters.events().is_empty());
}

#[test]
fn capture_rate_sets_labelled_gauge() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[("litellm_spend_capture_rate", &["api_provider"])]);
    set_spend_capture_rate(
        &counters,
        "openai",
        Some(finite("0.5").decimal()),
        &fixture.settings,
        EndUserCostTrackingSettings::default(),
    )
    .unwrap();
    let events = counters.events();
    let Recorded::Set {
        metric,
        labels,
        amount,
    } = &events[0]
    else {
        panic!("the capture rate gauge was not set");
    };
    assert_eq!(*metric, UsageMetric::SpendCaptureRate);
    assert_eq!(*amount, finite("0.5"));
    assert_eq!(
        labels.get("api_provider").map(String::as_str),
        Some("openai")
    );
}

#[test]
fn capture_rate_none_sets_nan() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[("litellm_spend_capture_rate", &["api_provider"])]);
    set_spend_capture_rate(
        &counters,
        "openai",
        None,
        &fixture.settings,
        EndUserCostTrackingSettings::default(),
    )
    .unwrap();
    let events = counters.events();
    let Recorded::Set { amount, .. } = &events[0] else {
        panic!("the capture rate gauge was not set");
    };
    assert_eq!(*amount, MetricValue::NotANumber);
}

#[test]
fn capture_rate_empty_label_mapping_sets_unlabelled() {
    let fixture = Fixture::with_end_user("alice");
    let counters = FakeCounters::with_labels(&[]);
    set_spend_capture_rate(
        &counters,
        "openai",
        Some(finite("0.5").decimal()),
        &fixture.settings,
        EndUserCostTrackingSettings::default(),
    )
    .unwrap();
    let events = counters.events();
    let Recorded::Set { labels, .. } = &events[0] else {
        panic!("the capture rate gauge was not set");
    };
    assert!(labels.is_empty());
}
