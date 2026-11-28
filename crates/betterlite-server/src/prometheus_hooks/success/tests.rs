#![cfg(test)]

use std::sync::Arc;

use betterlite_logging::callback_logger::CallbackLogger as _;
use betterlite_logging::callback_logger::error::LoggerError;
use betterlite_logging::prometheus_helpers::bounded_series_tracker::SeriesLimits;
use betterlite_logging::prometheus_rate_limits::RemainingModelGroupUsage;
use betterlite_models::decisions::Json;
use betterlite_models::key_digest::hashing::hash_token;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::super::tests::support::{
    API_KEY_ALLOWED, API_KEY_USED, CACHE_HITS, CACHED_TOKENS, DEPLOYMENT_PER_OUTPUT_TOKEN,
    DEPLOYMENT_STATE, DEPLOYMENT_SUCCESSES, DEPLOYMENT_TOTAL, IMAGES_GENERATED, INPUT_AUDIO_TOKENS,
    INPUT_CACHE_CREATION_TOKENS, INPUT_CACHED_TOKENS, INPUT_TOKENS, LATENCY, MCP_CALLS, MCP_SPEND,
    OUTPUT_AUDIO_TOKENS, OUTPUT_REASONING_TOKENS, OUTPUT_TOKENS, OVERHEAD_LATENCY,
    PROVIDER_CACHE_CREATION, PROVIDER_CACHE_READ, PROXY_TOTAL, QUEUE_TIME, REMAINING_REQUESTS,
    REMAINING_TOKENS, REQUESTS, RecordingRouterUsage, SPEND, TEAM_ALLOWED, TEAM_USED,
    TOTAL_LATENCY, TOTAL_TOKENS, TTFT, VIDEO_DURATION, VIRTUAL_KEY_REQUESTS, VIRTUAL_KEY_TOKENS,
    ZERO_COST, dec, error_information, event, event_with_metadata, fixture,
    fixture_with_input_sequence_length, fixture_with_limits, fixture_with_ports,
    fixture_with_settings, labelled_settings, labels_seen, number, observations, payload,
    rich_payload, series_label, totals,
};
use crate::prometheus_hooks::error::PrometheusHookError;
use crate::prometheus_hooks::hooks::PrometheusHookPorts;

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
async fn success_emits_every_step_and_ends_with_the_total() {
    let router = Arc::new(RecordingRouterUsage::new(9, 5));
    let usage: Arc<dyn RemainingModelGroupUsage> = Arc::clone(&router);
    let fixture = fixture_with_ports(
        labelled_settings(),
        PrometheusHookPorts {
            router_usage: Some(usage),
            requested_models: None,
        },
    );
    let metadata = IndexMap::from([
        (
            "litellm-key-remaining-requests-gpt-4-group".to_owned(),
            number("7"),
        ),
        (
            "litellm-key-remaining-tokens-gpt-4-group".to_owned(),
            number("5"),
        ),
        ("queue_time_seconds".to_owned(), number("1")),
    ]);
    fixture
        .hooks
        .async_log_success_event(Arc::new(event_with_metadata(
            rich_payload(dec("2.5")),
            metadata,
        )))
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
    // the budget fan-out ran on the hashed key
    assert_eq!(fixture.lookups.keys(), vec![hash_token("sk-live")]);
    // rate limits: the virtual-key gauges, then the key and team allowed/used pairs
    assert_eq!(totals(registry, VIRTUAL_KEY_REQUESTS), vec![dec("7")]);
    assert_eq!(totals(registry, VIRTUAL_KEY_TOKENS), vec![dec("5")]);
    assert_eq!(totals(registry, API_KEY_ALLOWED), vec![dec("10")]);
    assert_eq!(totals(registry, API_KEY_USED), vec![dec("4")]);
    assert_eq!(totals(registry, TEAM_ALLOWED), vec![dec("100")]);
    assert_eq!(totals(registry, TEAM_USED), vec![dec("30")]);
    // latency: first token, api latency, total latency and queue time, then the overhead histogram
    assert_eq!(observations(registry, TTFT), vec![1]);
    assert_eq!(observations(registry, LATENCY), vec![1]);
    assert_eq!(observations(registry, TOTAL_LATENCY), vec![1]);
    assert_eq!(observations(registry, QUEUE_TIME), vec![1]);
    assert_eq!(observations(registry, OVERHEAD_LATENCY), vec![1]);
    // deployment success and its state, then the router's remaining usage
    assert_eq!(totals(registry, DEPLOYMENT_SUCCESSES), vec![Decimal::ONE]);
    assert_eq!(totals(registry, DEPLOYMENT_TOTAL), vec![Decimal::ONE]);
    assert_eq!(totals(registry, DEPLOYMENT_STATE), vec![Decimal::ZERO]);
    assert_eq!(observations(registry, DEPLOYMENT_PER_OUTPUT_TOKEN), vec![1]);
    assert_eq!(totals(registry, REMAINING_REQUESTS), vec![dec("9")]);
    assert_eq!(totals(registry, REMAINING_TOKENS), vec![dec("5")]);
    assert_eq!(router.groups(), vec!["gpt-4-group".to_owned()]);
    // and the tail: cache, media, MCP calls and finally the proxy total
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
async fn success_labels_match_litellm() {
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
    assert_eq!(series_label(registry, SPEND, "service_tier"), "priority");
    // the proxy total carries the status, the route and the opt-in stream label
    assert_eq!(series_label(registry, PROXY_TOTAL, "status_code"), "200");
    assert_eq!(
        series_label(registry, PROXY_TOTAL, "route"),
        "/v1/chat/completions"
    );
    assert_eq!(series_label(registry, PROXY_TOTAL, "stream"), "True");
    // the deployment counters name the deployment, its api base and its group, and latency carries the tier
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
    // the failure-only labels are not part of the success schema
    let request_labels = registry
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
    let mut record = payload(Decimal::ZERO);
    record.model = "openrouter/anthropic/claude-3.5-sonnet".to_owned();
    record.custom_llm_provider = Some("openrouter".to_owned());
    routed
        .hooks
        .async_log_success_event(Arc::new(event(record)))
        .await
        .unwrap();
    assert_eq!(
        series_label(&routed.registry, REQUESTS, "model"),
        "anthropic/claude-3.5-sonnet"
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
    record.end_time = Decimal::MAX;
    let metadata = IndexMap::from([
        (
            "queue_time_seconds".to_owned(),
            number(&Decimal::MAX.to_string()),
        ),
        (
            "litellm-key-remaining-requests-gpt-4-group".to_owned(),
            number("7"),
        ),
        (
            "litellm-key-remaining-tokens-gpt-4-group".to_owned(),
            number("5"),
        ),
    ]);
    let result = fixture
        .hooks
        .async_log_success_event(Arc::new(event_with_metadata(record, metadata)))
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
