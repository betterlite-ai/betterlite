#![cfg(test)]

use std::ptr::{eq, from_ref};
use std::sync::Arc;

use betterlite_logging::callback_logger::CallbackLogger as _;
use betterlite_logging::callback_logger::error::LoggerError;
use betterlite_logging::prometheus_deployment::metrics::DeploymentMetrics;
use betterlite_logging::prometheus_helpers::bounded_series_tracker::SeriesLimits;
use betterlite_logging::prometheus_latency::input_sequence_length::UsageMapping;
use betterlite_logging::prometheus_rate_limits::gauge::RateLimitGauges;
use betterlite_logging::prometheus_usage::counters::UsageCounters;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::key_digest::hashing::hash_token;
use betterlite_models::zero_cost::{TEXT_INPUT_RATE, ZeroCostReason};
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::super::tests::support::{
    API_KEY_ALLOWED, API_KEY_USED, CACHE_HITS, CACHED_TOKENS, DEPLOYMENT_PER_OUTPUT_TOKEN,
    DEPLOYMENT_STATE, DEPLOYMENT_SUCCESSES, DEPLOYMENT_TOTAL, IMAGES_GENERATED, INPUT_AUDIO_TOKENS,
    INPUT_CACHE_CREATION_TOKENS, INPUT_CACHED_TOKENS, INPUT_TOKENS, KEY_BUDGET, LATENCY, MCP_CALLS,
    MCP_SPEND, OUTPUT_AUDIO_TOKENS, OUTPUT_REASONING_TOKENS, OUTPUT_TOKENS, OVERHEAD_LATENCY,
    PROVIDER_CACHE_CREATION, PROVIDER_CACHE_READ, PROXY_TOTAL, QUEUE_TIME, REMAINING_REQUESTS,
    REMAINING_TOKENS, REQUESTS, SPEND, TEAM_ALLOWED, TEAM_USED, TOTAL_LATENCY, TOTAL_TOKENS, TTFT,
    VIDEO_DURATION, VIRTUAL_KEY_REQUESTS, VIRTUAL_KEY_TOKENS, ZERO_COST, dec, error_information,
    event, event_with_metadata, fixture, fixture_with_input_sequence_length, fixture_with_limits,
    fixture_with_settings, labelled_settings, labels_seen, number, observations, payload,
    rich_payload, router_hooks, series_label, success_metadata, totals,
};
use crate::prometheus_hooks::error::PrometheusHookError;
use crate::prometheus_hooks::event::HookCall;
use crate::prometheus_hooks::json::{as_object, input_length_inputs};
use crate::prometheus_hooks::labels::{hashed_key, zero_cost_diagnostic};
use crate::prometheus_hooks::latency::now_seconds;

#[tokio::test]
async fn missing_payload_is_an_error() {
    let fixture = fixture();
    let event = event(payload(Decimal::ZERO));
    let result = fixture.hooks.log_success(None, &event).await;
    match result {
        Err(PrometheusHookError::MissingPayload) => {
            assert_eq!(
                PrometheusHookError::MissingPayload.to_string(),
                "standard_logging_object is required, got=None"
            );
        }
        Err(other) => panic!("expected the missing-payload error, got {other}"),
        Ok(()) => panic!("expected the missing-payload error, got Ok"),
    }
    assert!(totals(&fixture.registry, REQUESTS).is_empty());
}

#[tokio::test]
async fn invalid_key_is_skipped() {
    let fixture = fixture();
    let mut record = payload(Decimal::ZERO);
    record.error_information = Some(error_information("Invalid API key passed"));
    fixture
        .hooks
        .async_log_success_event(Arc::new(event(record)))
        .await
        .unwrap();
    assert!(totals(&fixture.registry, REQUESTS).is_empty());
    assert!(totals(&fixture.registry, PROXY_TOTAL).is_empty());
}

#[tokio::test]
async fn success_emits_usage_and_token_metrics() {
    let fixture = fixture_with_settings(labelled_settings());
    fixture
        .hooks
        .async_log_success_event(Arc::new(event(rich_payload(dec("2.5")))))
        .await
        .unwrap();
    let registry = &fixture.registry;
    // usage: requests and spend, the zero-cost counter, then the token counters and their details
    assert_eq!(totals(registry, REQUESTS), vec![Decimal::ONE]);
    assert_eq!(totals(registry, SPEND), vec![dec("2.5")]);
    assert_eq!(totals(registry, ZERO_COST), vec![Decimal::ONE]);
    assert_eq!(totals(registry, TOTAL_TOKENS), vec![dec("12")]);
    assert_eq!(totals(registry, INPUT_TOKENS), vec![dec("8")]);
    assert_eq!(totals(registry, OUTPUT_TOKENS), vec![dec("4")]);
    assert_eq!(totals(registry, INPUT_CACHED_TOKENS), vec![dec("3")]);
    assert_eq!(
        totals(registry, INPUT_CACHE_CREATION_TOKENS),
        vec![Decimal::ONE]
    );
    assert_eq!(totals(registry, INPUT_AUDIO_TOKENS), vec![Decimal::ONE]);
    assert_eq!(totals(registry, OUTPUT_REASONING_TOKENS), vec![dec("2")]);
    assert_eq!(totals(registry, OUTPUT_AUDIO_TOKENS), vec![Decimal::ONE]);
}

#[tokio::test]
async fn success_emits_the_budget_and_rate_limit_gauges() {
    let (fixture, _router) = router_hooks();
    fixture
        .hooks
        .async_log_success_event(Arc::new(event_with_metadata(
            rich_payload(dec("2.5")),
            success_metadata(),
        )))
        .await
        .unwrap();
    let registry = &fixture.registry;
    // the budget fan-out ran on the hashed key
    assert_eq!(fixture.lookups.keys(), vec![hash_token("sk-live")]);
    // rate limits: the virtual-key gauges, then the key and team allowed/used pairs
    assert_eq!(totals(registry, VIRTUAL_KEY_REQUESTS), vec![dec("7")]);
    assert_eq!(totals(registry, VIRTUAL_KEY_TOKENS), vec![dec("5")]);
    assert_eq!(totals(registry, API_KEY_ALLOWED), vec![dec("10")]);
    assert_eq!(totals(registry, API_KEY_USED), vec![dec("4")]);
    assert_eq!(totals(registry, TEAM_ALLOWED), vec![dec("100")]);
    assert_eq!(totals(registry, TEAM_USED), vec![dec("30")]);
}

#[tokio::test]
async fn success_emits_latency_and_deployment_metrics() {
    let (fixture, router) = router_hooks();
    fixture
        .hooks
        .async_log_success_event(Arc::new(event_with_metadata(
            rich_payload(dec("2.5")),
            success_metadata(),
        )))
        .await
        .unwrap();
    let registry = &fixture.registry;
    // latency: first token and api latency, then the total with the queue time added and the queue histogram
    assert_eq!(observations(registry, TTFT), vec![1]);
    assert_eq!(observations(registry, LATENCY), vec![1]);
    assert_eq!(totals(registry, TOTAL_LATENCY), vec![dec("4")]);
    assert_eq!(totals(registry, QUEUE_TIME), vec![dec("1")]);
    assert_eq!(observations(registry, OVERHEAD_LATENCY), vec![1]);
    // deployment success and its state, then the router's remaining usage
    assert_eq!(totals(registry, DEPLOYMENT_SUCCESSES), vec![Decimal::ONE]);
    assert_eq!(totals(registry, DEPLOYMENT_TOTAL), vec![Decimal::ONE]);
    // a streaming success measures from the completion start: one second over four output tokens
    assert_eq!(
        totals(registry, DEPLOYMENT_PER_OUTPUT_TOKEN),
        vec![dec("0.25")]
    );
    assert_eq!(totals(registry, DEPLOYMENT_STATE), vec![Decimal::ZERO]);
    assert_eq!(totals(registry, REMAINING_REQUESTS), vec![dec("9")]);
    assert_eq!(totals(registry, REMAINING_TOKENS), vec![dec("5")]);
    assert_eq!(router.groups(), vec!["gpt-4-group".to_owned()]);
}

#[tokio::test]
async fn success_emits_the_cache_media_and_mcp_metrics_last() {
    let fixture = fixture_with_settings(labelled_settings());
    fixture
        .hooks
        .async_log_success_event(Arc::new(event(rich_payload(dec("2.5")))))
        .await
        .unwrap();
    let registry = &fixture.registry;
    // the tail: cache, media, MCP calls and finally the proxy total
    assert_eq!(totals(registry, CACHE_HITS), vec![Decimal::ONE]);
    assert_eq!(totals(registry, CACHED_TOKENS), vec![dec("12")]);
    assert_eq!(totals(registry, PROVIDER_CACHE_READ), vec![dec("3")]);
    assert_eq!(
        totals(registry, PROVIDER_CACHE_CREATION),
        vec![Decimal::ONE]
    );
    assert_eq!(totals(registry, VIDEO_DURATION), vec![dec("2.5")]);
    assert_eq!(totals(registry, IMAGES_GENERATED), vec![dec("2")]);
    assert_eq!(totals(registry, MCP_CALLS), vec![Decimal::ONE]);
    assert_eq!(totals(registry, MCP_SPEND), vec![dec("2.5")]);
    assert_eq!(totals(registry, PROXY_TOTAL), vec![Decimal::ONE]);
}

#[tokio::test]
async fn success_request_labels_match_litellm() {
    let fixture = fixture_with_settings(labelled_settings());
    fixture
        .hooks
        .async_log_success_event(Arc::new(event(rich_payload(dec("2.5")))))
        .await
        .unwrap();
    let registry = &fixture.registry;
    assert_eq!(series_label(registry, REQUESTS, "model"), "gpt-4o");
    assert_eq!(
        series_label(registry, REQUESTS, "requested_model"),
        "gpt-4-group"
    );
    assert_eq!(series_label(registry, REQUESTS, "model_id"), "deployment-1");
    assert_eq!(series_label(registry, REQUESTS, "api_provider"), "openai");
    assert_eq!(series_label(registry, REQUESTS, "end_user"), "end-user-1");
    assert_eq!(series_label(registry, REQUESTS, "user"), "user-1");
    assert_eq!(
        series_label(registry, REQUESTS, "hashed_api_key"),
        "sk-live"
    );
    assert_eq!(series_label(registry, REQUESTS, "api_key_alias"), "alias-1");
    assert_eq!(series_label(registry, REQUESTS, "team"), "team-1");
    assert_eq!(series_label(registry, REQUESTS, "team_alias"), "team-alias");
    assert_eq!(
        series_label(registry, REQUESTS, "user_email"),
        "user@example.com"
    );
    assert_eq!(series_label(registry, REQUESTS, "client_ip"), "10.0.0.7");
    assert_eq!(series_label(registry, REQUESTS, "user_agent"), "test-agent");
    assert_eq!(series_label(registry, REQUESTS, "org_id"), "org-1");
    assert_eq!(series_label(registry, REQUESTS, "org_alias"), "org-alias");
    assert_eq!(series_label(registry, REQUESTS, "tag_prod"), "true");
}

#[tokio::test]
async fn success_deployment_and_total_labels_match_litellm() {
    let fixture = fixture_with_settings(labelled_settings());
    fixture
        .hooks
        .async_log_success_event(Arc::new(event(rich_payload(dec("2.5")))))
        .await
        .unwrap();
    let registry = &fixture.registry;
    // the proxy total carries the status, the route and the opt-in stream label
    assert_eq!(series_label(registry, PROXY_TOTAL, "status_code"), "200");
    assert_eq!(
        series_label(registry, PROXY_TOTAL, "route"),
        "/v1/chat/completions"
    );
    assert_eq!(series_label(registry, PROXY_TOTAL, "stream"), "True");
    // the deployment counters name the deployment, its api base and its group, and the tier reaches spend and latency
    assert_eq!(
        series_label(registry, DEPLOYMENT_TOTAL, "model_group"),
        "gpt-4-group"
    );
    assert_eq!(
        series_label(registry, DEPLOYMENT_TOTAL, "requested_model"),
        "gpt-4-group"
    );
    assert_eq!(
        series_label(registry, DEPLOYMENT_TOTAL, "litellm_model_name"),
        "gpt-4o"
    );
    assert_eq!(
        series_label(registry, DEPLOYMENT_TOTAL, "api_base"),
        "https://api.example/v1"
    );
    assert_eq!(series_label(registry, LATENCY, "service_tier"), "priority");
    assert_eq!(series_label(registry, LATENCY, "model"), "gpt-4o");
    assert_eq!(series_label(registry, SPEND, "service_tier"), "priority");
}

#[tokio::test]
async fn the_success_schema_has_no_failure_only_labels() {
    let fixture = fixture();
    // the failure-only labels are not part of the success schema
    let request_labels = fixture
        .registry
        .get(REQUESTS)
        .expect("the requests metric is registered")
        .description
        .label_names
        .clone();
    assert!(!request_labels.contains(&"status_code".to_owned()));
    assert!(!request_labels.contains(&"litellm_model_name".to_owned()));
}

#[tokio::test]
async fn a_bedrock_model_is_labelled_without_the_provider_prefix() {
    let fixture = fixture();
    let mut record = payload(Decimal::ZERO);
    record.model = "bedrock/claude-3".to_owned();
    record.custom_llm_provider = Some("bedrock".to_owned());
    record.model_id = Some("deployment-1".to_owned());
    fixture
        .hooks
        .async_log_success_event(Arc::new(event(record)))
        .await
        .unwrap();
    let registry = &fixture.registry;
    assert_eq!(series_label(registry, REQUESTS, "model"), "claude-3");
    assert_eq!(
        series_label(registry, DEPLOYMENT_TOTAL, "litellm_model_name"),
        "claude-3"
    );
    assert_eq!(series_label(registry, LATENCY, "model"), "claude-3");
}

#[tokio::test]
async fn an_openrouter_native_id_keeps_its_prefix() {
    let native = fixture();
    let mut record = payload(Decimal::ZERO);
    record.model = "openrouter/auto".to_owned();
    record.custom_llm_provider = Some("openrouter".to_owned());
    native
        .hooks
        .async_log_success_event(Arc::new(event(record)))
        .await
        .unwrap();
    assert_eq!(
        series_label(&native.registry, REQUESTS, "model"),
        "openrouter/auto"
    );

    let routed = fixture();
    let mut routed_record = payload(Decimal::ZERO);
    routed_record.model = "openrouter/anthropic/claude-3.5-sonnet".to_owned();
    routed_record.custom_llm_provider = Some("openrouter".to_owned());
    routed
        .hooks
        .async_log_success_event(Arc::new(event(routed_record)))
        .await
        .unwrap();
    assert_eq!(
        series_label(&routed.registry, REQUESTS, "model"),
        "anthropic/claude-3.5-sonnet"
    );
}

#[tokio::test]
async fn a_cohere_chat_model_is_labelled_without_the_declared_prefix() {
    let fixture = fixture_with_settings(labelled_settings());
    let mut record = rich_payload(dec("2.5"));
    record.model = "cohere/command-r".to_owned();
    record.custom_llm_provider = Some("cohere_chat".to_owned());
    fixture
        .hooks
        .async_log_success_event(Arc::new(event(record)))
        .await
        .unwrap();
    let registry = &fixture.registry;
    assert_eq!(series_label(registry, REQUESTS, "model"), "command-r");
    assert_eq!(
        series_label(registry, DEPLOYMENT_SUCCESSES, "litellm_model_name"),
        "command-r"
    );
}

#[tokio::test]
async fn an_anthropic_text_model_is_labelled_without_the_declared_prefix() {
    let fixture = fixture_with_settings(labelled_settings());
    let mut record = rich_payload(dec("2.5"));
    record.model = "anthropic/claude-2".to_owned();
    record.custom_llm_provider = Some("anthropic_text".to_owned());
    fixture
        .hooks
        .async_log_success_event(Arc::new(event(record)))
        .await
        .unwrap();
    let registry = &fixture.registry;
    assert_eq!(series_label(registry, REQUESTS, "model"), "claude-2");
    assert_eq!(
        series_label(registry, DEPLOYMENT_SUCCESSES, "litellm_model_name"),
        "claude-2"
    );
}

#[tokio::test]
async fn an_ai21_chat_model_is_labelled_without_the_declared_prefix() {
    let fixture = fixture_with_settings(labelled_settings());
    let mut record = rich_payload(dec("2.5"));
    record.model = "ai21/jamba-instruct".to_owned();
    record.custom_llm_provider = Some("ai21_chat".to_owned());
    fixture
        .hooks
        .async_log_success_event(Arc::new(event(record)))
        .await
        .unwrap();
    let registry = &fixture.registry;
    assert_eq!(series_label(registry, REQUESTS, "model"), "jamba-instruct");
    assert_eq!(
        series_label(registry, DEPLOYMENT_SUCCESSES, "litellm_model_name"),
        "jamba-instruct"
    );
}

#[tokio::test]
async fn a_model_prefixed_with_its_renamed_provider_loses_that_prefix() {
    let fixture = fixture_with_settings(labelled_settings());
    let mut record = rich_payload(dec("2.5"));
    record.model = "cohere_chat/command-r".to_owned();
    record.custom_llm_provider = Some("cohere_chat".to_owned());
    fixture
        .hooks
        .async_log_success_event(Arc::new(event(record)))
        .await
        .unwrap();
    assert_eq!(
        series_label(&fixture.registry, REQUESTS, "model"),
        "command-r"
    );
}

#[tokio::test]
async fn an_undeclared_cohere_chat_model_is_labelled_without_its_cohere_prefix() {
    let fixture = fixture_with_settings(labelled_settings());
    let mut record = rich_payload(dec("2.5"));
    record.model = "cohere/command-r-08-2024".to_owned();
    record.custom_llm_provider = None;
    fixture
        .hooks
        .async_log_success_event(Arc::new(event(record)))
        .await
        .unwrap();
    assert_eq!(
        series_label(&fixture.registry, REQUESTS, "model"),
        "command-r-08-2024"
    );
}

#[tokio::test]
async fn an_undeclared_anthropic_text_model_is_labelled_without_its_anthropic_prefix() {
    let fixture = fixture_with_settings(labelled_settings());
    let mut record = rich_payload(dec("2.5"));
    record.model = "anthropic/claude-2".to_owned();
    record.custom_llm_provider = None;
    fixture
        .hooks
        .async_log_success_event(Arc::new(event(record)))
        .await
        .unwrap();
    assert_eq!(
        series_label(&fixture.registry, REQUESTS, "model"),
        "claude-2"
    );
}

#[tokio::test]
async fn a_model_without_a_resolved_provider_is_labelled_whole() {
    let fixture = fixture_with_settings(labelled_settings());
    let mut record = rich_payload(dec("2.5"));
    record.model = "/gpt-4o".to_owned();
    record.custom_llm_provider = None;
    fixture
        .hooks
        .async_log_success_event(Arc::new(event(record)))
        .await
        .unwrap();
    assert_eq!(
        series_label(&fixture.registry, REQUESTS, "model"),
        "/gpt-4o"
    );
}

#[tokio::test]
async fn deployment_remaining_headers_set_the_gauges() {
    let fixture = fixture();
    let mut record = rich_payload(dec("2.5"));
    record.hidden_params.additional_headers = Some(IndexMap::from([
        ("x_ratelimit_remaining_requests".to_owned(), number("9")),
        ("x_ratelimit_remaining_tokens".to_owned(), number("5")),
    ]));
    fixture
        .hooks
        .async_log_success_event(Arc::new(event(record)))
        .await
        .unwrap();
    assert_eq!(
        totals(&fixture.registry, REMAINING_REQUESTS),
        vec![dec("9")]
    );
    assert_eq!(totals(&fixture.registry, REMAINING_TOKENS), vec![dec("5")]);
}

#[tokio::test]
async fn sk_key_is_hashed_for_the_emitters_only() {
    let fixture = fixture();
    let mut record = payload(Decimal::ZERO);
    record.metadata.user_api_key_hash = Some("sk-abc".to_owned());
    fixture
        .hooks
        .async_log_success_event(Arc::new(event(record)))
        .await
        .unwrap();
    let hashed = hash_token("sk-abc");
    let request_labels = labels_seen(&fixture.registry, REQUESTS);
    assert!(request_labels.contains(&"sk-abc".to_owned()));
    assert!(!request_labels.contains(&hashed));
    let virtual_key_labels = labels_seen(&fixture.registry, VIRTUAL_KEY_REQUESTS);
    assert!(virtual_key_labels.contains(&hashed));
    assert!(!virtual_key_labels.contains(&"sk-abc".to_owned()));
    assert_eq!(fixture.lookups.keys(), vec![hashed]);
}

#[tokio::test]
async fn the_virtual_key_error_stops_the_rate_limits_and_the_later_steps() {
    let fixture = fixture_with_settings(labelled_settings());
    let metadata = IndexMap::from([(
        "litellm-key-remaining-requests-gpt-4-group".to_owned(),
        Json::Text("not-a-number".to_owned()),
    )]);
    let result = fixture
        .hooks
        .async_log_success_event(Arc::new(event_with_metadata(
            rich_payload(dec("2.5")),
            metadata,
        )))
        .await;
    assert!(matches!(result, Err(LoggerError::Delivery { .. })));
    let registry = &fixture.registry;
    // the whole usage phase ran before the rate-limit emitter
    assert_eq!(totals(registry, REQUESTS), vec![Decimal::ONE]);
    assert_eq!(totals(registry, SPEND), vec![dec("2.5")]);
    assert_eq!(totals(registry, ZERO_COST), vec![Decimal::ONE]);
    assert_eq!(totals(registry, TOTAL_TOKENS), vec![dec("12")]);
    assert_eq!(totals(registry, INPUT_TOKENS), vec![dec("8")]);
    assert_eq!(totals(registry, OUTPUT_TOKENS), vec![dec("4")]);
    // the virtual-key lookup failed before either of its gauges was written
    assert!(totals(registry, VIRTUAL_KEY_REQUESTS).is_empty());
    assert!(totals(registry, VIRTUAL_KEY_TOKENS).is_empty());
    // and no key/team, latency, deployment, router or tail step ran
    assert!(totals(registry, API_KEY_ALLOWED).is_empty());
    assert!(totals(registry, API_KEY_USED).is_empty());
    assert!(totals(registry, TEAM_ALLOWED).is_empty());
    assert!(totals(registry, TEAM_USED).is_empty());
    assert!(observations(registry, LATENCY).is_empty());
    assert!(observations(registry, TOTAL_LATENCY).is_empty());
    assert!(totals(registry, DEPLOYMENT_SUCCESSES).is_empty());
    assert!(totals(registry, DEPLOYMENT_TOTAL).is_empty());
    assert!(totals(registry, REMAINING_REQUESTS).is_empty());
    assert!(totals(registry, CACHE_HITS).is_empty());
    assert!(totals(registry, MCP_CALLS).is_empty());
    assert!(totals(registry, PROXY_TOTAL).is_empty());
}

#[tokio::test]
async fn a_latency_overflow_stops_the_deployment_and_the_tail() {
    let fixture = fixture_with_settings(labelled_settings());
    let mut record = rich_payload(dec("2.5"));
    // the total alone sits at the ceiling, so adding the metadata's one queue-time second overflows
    record.start_time = Decimal::ZERO;
    record.end_time = Decimal::MAX;
    let result = fixture
        .hooks
        .async_log_success_event(Arc::new(event_with_metadata(record, success_metadata())))
        .await;
    assert!(matches!(result, Err(LoggerError::Delivery { .. })));
    let registry = &fixture.registry;
    // usage and the rate limits ran
    assert_eq!(totals(registry, REQUESTS), vec![Decimal::ONE]);
    assert_eq!(totals(registry, ZERO_COST), vec![Decimal::ONE]);
    assert_eq!(totals(registry, VIRTUAL_KEY_REQUESTS), vec![dec("7")]);
    assert_eq!(totals(registry, VIRTUAL_KEY_TOKENS), vec![dec("5")]);
    assert_eq!(totals(registry, API_KEY_ALLOWED), vec![dec("10")]);
    // the first token and api-latency histograms ran, then the total plus the queue time overflowed
    assert_eq!(observations(registry, TTFT), vec![1]);
    assert_eq!(observations(registry, LATENCY), vec![1]);
    assert!(observations(registry, TOTAL_LATENCY).is_empty());
    assert!(observations(registry, QUEUE_TIME).is_empty());
    // and no deployment, router or tail step ran
    assert!(totals(registry, DEPLOYMENT_SUCCESSES).is_empty());
    assert!(totals(registry, DEPLOYMENT_TOTAL).is_empty());
    assert!(totals(registry, REMAINING_REQUESTS).is_empty());
    assert!(totals(registry, REMAINING_TOKENS).is_empty());
    assert!(totals(registry, CACHE_HITS).is_empty());
    assert!(totals(registry, MCP_CALLS).is_empty());
    assert!(totals(registry, PROXY_TOTAL).is_empty());
}

#[tokio::test]
async fn the_series_limit_evicts_the_oldest_end_user_series() {
    let fixture = fixture_with_limits(SeriesLimits {
        max_series: Some(1),
        ttl: None,
        cleanup_interval: None,
    });
    let first = IndexMap::from([(
        "user_api_key_end_user_id".to_owned(),
        Json::Text("user-a".to_owned()),
    )]);
    fixture
        .hooks
        .async_log_success_event(Arc::new(event_with_metadata(payload(Decimal::ZERO), first)))
        .await
        .unwrap();
    let second = IndexMap::from([(
        "user_api_key_end_user_id".to_owned(),
        Json::Text("user-b".to_owned()),
    )]);
    fixture
        .hooks
        .async_log_success_event(Arc::new(event_with_metadata(
            payload(Decimal::ZERO),
            second,
        )))
        .await
        .unwrap();
    let collector = fixture
        .registry
        .get(REQUESTS)
        .expect("the requests metric is registered");
    let series = collector.series_in_use_order();
    let (labels, _sample) = series.first().expect("the requests metric has a series");
    // the cap keeps the newer end-user series and drops the older one from the registry
    assert_eq!(series.len(), 1);
    assert!(labels.contains(&"user-b".to_owned()));
}

#[tokio::test]
async fn the_input_sequence_length_label_reads_the_metadata_usage() {
    let fixture = fixture_with_input_sequence_length();
    let mut record = payload(Decimal::ZERO);
    record.response = Some(Json::Object(IndexMap::from([(
        "usage".to_owned(),
        Json::Object(IndexMap::new()),
    )])));
    record.metadata.usage_object = Some(Json::Object(IndexMap::from([(
        "prompt_tokens".to_owned(),
        number("2500"),
    )])));
    fixture
        .hooks
        .async_log_success_event(Arc::new(event(record)))
        .await
        .unwrap();
    // with no prompt tokens on the payload the reader falls to the usage object the metadata carries
    assert_eq!(
        series_label(&fixture.registry, LATENCY, "input_sequence_length"),
        "1k-4k"
    );
}

#[tokio::test]
async fn the_budget_fan_out_reads_the_request_metadata() {
    let fixture = fixture_with_settings(labelled_settings());
    let metadata = IndexMap::from([
        ("user_api_key_spend".to_owned(), number("4")),
        ("user_api_key_max_budget".to_owned(), number("10")),
    ]);
    fixture
        .hooks
        .async_log_success_event(Arc::new(event_with_metadata(
            rich_payload(dec("2.5")),
            metadata,
        )))
        .await
        .unwrap();
    // the fan-out sees the carried spend and the max budget, so the key gauge keeps 10 - (4 + 2.5)
    assert_eq!(totals(&fixture.registry, KEY_BUDGET), vec![dec("3.5")]);
}

#[tokio::test]
async fn success_fans_out_every_budget_entity() {
    let fixture = fixture_with_settings(labelled_settings());
    fixture
        .hooks
        .async_log_success_event(Arc::new(event(rich_payload(dec("2.5")))))
        .await
        .unwrap();
    // one lookup per entity, each with the id the label values carry
    assert_eq!(fixture.lookups.keys(), vec![hash_token("sk-live")]);
    assert_eq!(fixture.lookups.teams(), vec!["team-1".to_owned()]);
    assert_eq!(fixture.lookups.users(), vec!["user-1".to_owned()]);
    assert_eq!(fixture.lookups.orgs(), vec!["org-1".to_owned()]);
    assert_eq!(fixture.lookups.customers(), vec!["end-user-1".to_owned()]);
}

#[test]
fn the_metrics_adapter_forwards_the_configured_ports() {
    let fixture = fixture_with_limits(SeriesLimits {
        max_series: Some(2),
        ttl: None,
        cleanup_interval: None,
    });
    let metrics = fixture.hooks.metrics();
    assert_eq!(
        UsageCounters::limits(&metrics),
        &fixture.hooks.settings.limits
    );
    assert_eq!(
        DeploymentMetrics::limits(&metrics),
        &fixture.hooks.settings.limits
    );
    assert!(eq(
        from_ref(UsageCounters::tracker(&metrics)),
        from_ref(&fixture.hooks.collectors.bounded_series_tracker)
    ));
    assert!(eq(
        from_ref(RateLimitGauges::tracker(&metrics)),
        from_ref(&fixture.hooks.collectors.bounded_series_tracker)
    ));
    assert_eq!(
        DeploymentMetrics::settings(&metrics),
        &fixture.hooks.settings.labels
    );
    assert_eq!(
        DeploymentMetrics::end_user_settings(&metrics),
        fixture.hooks.settings.end_user
    );
}

#[test]
fn the_rebuilt_kwargs_carry_the_payload_identity() {
    let call = HookCall::from_event(&event(rich_payload(dec("2.5"))), &ModelCatalog::default());
    assert_eq!(call.model, "gpt-4o");
    assert_eq!(
        call.metadata.get("user_api_key_user_id"),
        Some(&Json::Text("user-1".to_owned()))
    );
    assert_eq!(
        call.metadata.get("model_group"),
        Some(&Json::Text("gpt-4-group".to_owned()))
    );
    let model_info = call
        .metadata
        .get("model_info")
        .and_then(as_object)
        .expect("the payload model id becomes model_info.id");
    assert_eq!(
        model_info.get("id"),
        Some(&Json::Text("deployment-1".to_owned()))
    );
}

#[test]
fn the_zero_cost_diagnostic_keeps_only_the_known_rate_keys() {
    let mut record = payload(Decimal::ZERO);
    record.zero_cost_diagnostic = Some(Json::Object(IndexMap::from([
        (
            "reason".to_owned(),
            Json::Text("missing_pricing_key".to_owned()),
        ),
        (
            "missing_pricing_keys".to_owned(),
            Json::List(vec![
                Json::Text("not_a_rate_key".to_owned()),
                Json::Text(TEXT_INPUT_RATE.to_owned()),
            ]),
        ),
    ])));
    let diagnostic = zero_cost_diagnostic(&record).expect("the diagnostic parses");
    assert_eq!(diagnostic.reason, ZeroCostReason::MissingPricingKey);
    assert_eq!(diagnostic.missing_pricing_keys, vec![TEXT_INPUT_RATE]);
}

#[test]
fn the_input_length_inputs_read_the_usage_object() {
    let mut record = payload(Decimal::ZERO);
    record.response = Some(Json::Object(IndexMap::from([(
        "usage".to_owned(),
        Json::Object(IndexMap::new()),
    )])));
    record.metadata.usage_object = Some(Json::Object(IndexMap::from([(
        "prompt_tokens".to_owned(),
        number("2500"),
    )])));
    let inputs = input_length_inputs(&record);
    assert!(inputs.response_has_usage);
    assert_eq!(
        inputs.metadata_usage,
        UsageMapping::Mapping {
            prompt_tokens: Some(2500_i64)
        }
    );
}

#[test]
fn a_key_without_the_sk_prefix_keeps_its_text() {
    assert_eq!(hashed_key(Some("plain-key")), Some("plain-key".to_owned()));
    assert_eq!(hashed_key(None), None);
}

#[test]
fn now_seconds_reads_the_current_epoch() {
    assert!(now_seconds() > Decimal::from(1_700_000_000_i64));
}

#[tokio::test]
async fn the_response_service_tier_wins_over_the_usage_object() {
    let fixture = fixture_with_settings(labelled_settings());
    let mut record = rich_payload(dec("2.5"));
    record.response = Some(Json::Object(IndexMap::from([(
        "service_tier".to_owned(),
        Json::Text("economy".to_owned()),
    )])));
    fixture
        .hooks
        .async_log_success_event(Arc::new(event(record)))
        .await
        .unwrap();
    assert_eq!(
        series_label(&fixture.registry, SPEND, "service_tier"),
        "economy"
    );
}

#[tokio::test]
async fn a_configured_custom_metadata_label_reaches_the_series() {
    let mut settings = labelled_settings();
    settings.labels.custom_metadata_labels = vec!["user_api_key_user_id".to_owned()];
    let fixture = fixture_with_settings(settings);
    fixture
        .hooks
        .async_log_success_event(Arc::new(event(rich_payload(dec("2.5")))))
        .await
        .unwrap();
    assert_eq!(
        series_label(&fixture.registry, REQUESTS, "user_api_key_user_id"),
        "user-1"
    );
}
