#![cfg(test)]

mod baggage {
    use betterlite_logging::otel::logger::call_context::seed_identity_baggage;
    use opentelemetry::Context;
    use opentelemetry::baggage::BaggageExt as _;

    use crate::otel::support::{identity, with_exporters};

    #[test]
    fn baggage_promotes_only_the_allowlisted_identity_keys() {
        let mut settings = with_exporters(Vec::new());
        settings.baggage_promoted_keys = vec!["litellm.team.id".to_owned()];
        settings.baggage_metadata_keys = vec!["route".to_owned()];
        let context = seed_identity_baggage(&settings, &identity(), Some("gpt-4o"), Context::new());
        let bag = context.baggage();
        assert_eq!(
            bag.get("litellm.team.id").map(|(value, _)| value.as_str()),
            Some("team-a")
        );
        assert_eq!(
            bag.get("litellm.metadata.route")
                .map(|(value, _)| value.as_str()),
            Some("chat")
        );
        // present on the identity, but not on either allowlist
        assert!(bag.get("litellm.api_key.hash").is_none());
        assert!(bag.get("gen_ai.request.model").is_none());
    }
}

mod dispatch {
    use crate::otel::support::{
        anchored, event, failure, finished, harness, inference, mcp_list, mcp_tool,
        mcp_tool_failure, names, owned_http, success, with_id,
    };

    #[tokio::test]
    async fn dispatch_emits_one_mcp_tool_span_and_skips_the_llm_path() {
        let harness = harness(owned_http(), anchored());
        success(&harness.logger, event(with_id(mcp_tool(), "call-mcp"))).await;
        let spans = finished(&harness.spans);
        assert_eq!(spans.len(), 1);
        assert_eq!(
            names(&spans).first().map(String::as_str),
            Some("tools/call search")
        );
    }

    #[tokio::test]
    async fn dispatch_emits_one_mcp_list_span_for_a_list_tools_call() {
        let harness = harness(owned_http(), anchored());
        success(&harness.logger, event(with_id(mcp_list(), "call-list"))).await;
        let spans = finished(&harness.spans);
        assert_eq!(spans.len(), 1);
        assert_eq!(
            names(&spans).first().map(String::as_str),
            Some("tools/list")
        );
    }

    #[tokio::test]
    async fn dispatch_emits_mcp_spans_on_the_failure_event_too() {
        let harness = harness(owned_http(), anchored());
        failure(
            &harness.logger,
            event(with_id(mcp_tool_failure(), "call-mcp")),
        )
        .await;
        let spans = finished(&harness.spans);
        assert_eq!(spans.len(), 1);
        assert_eq!(
            names(&spans).first().map(String::as_str),
            Some("tools/call search")
        );
    }

    #[tokio::test]
    async fn dispatch_falls_through_to_the_llm_path_for_an_inference_payload() {
        let harness = harness(owned_http(), anchored());
        success(&harness.logger, event(with_id(inference(), "call-a"))).await;
        let spans = finished(&harness.spans);
        assert_eq!(spans.len(), 1);
        assert!(
            names(&spans)
                .first()
                .is_some_and(|name| name.contains("gpt-4o"))
        );
    }
}

mod finish {
    use crate::otel::support::{
        anchored, attribute, event, finished, harness, owned_http, payload_start, pre_call,
        success, team_payload, with_id,
    };

    #[tokio::test]
    async fn finish_renames_the_boundary_span_with_the_payload_model() {
        let harness = harness(owned_http(), anchored());
        pre_call(&harness.logger, "call-a", "other-model", false, None, None);
        success(&harness.logger, event(with_id(inference(), "call-a"))).await;
        let spans = finished(&harness.spans);
        assert_eq!(spans.len(), 1);
        let span = spans.first().expect("span");
        assert!(span.name.contains("gpt-4o"));
        assert!(!span.name.contains("other-model"));
        // born at the boundary, not at close: only the live span keeps the pre_call instant
        assert_ne!(span.start_time, payload_start());
    }

    #[tokio::test]
    async fn finish_emits_a_deferred_span_seeded_with_identity_baggage() {
        let mut settings = owned_http();
        settings.baggage_promoted_keys = vec!["litellm.team.id".to_owned()];
        let harness = harness(settings, anchored());
        success(&harness.logger, event(with_id(team_payload(), "call-team"))).await;
        let spans = finished(&harness.spans);
        assert_eq!(spans.len(), 1);
        let span = spans.first().expect("span");
        assert_eq!(
            attribute(span, "litellm.team.id").as_deref(),
            Some("team-a")
        );
        assert_eq!(span.start_time, payload_start());
    }
}

mod llm_close {
    use betterlite_models::decisions::Json;

    use crate::otel::support::{
        anchored, event, event_with, failure, finished, harness, inference, inference_failure,
        no_upstream, owned_http, payload_start, pre_call, success, with_id, zero_start,
    };

    #[tokio::test]
    async fn llm_close_dedups_the_second_callback_for_one_call_id() {
        let harness = harness(owned_http(), anchored());
        pre_call(&harness.logger, "call-a", "gpt-4o", false, None, None);
        success(&harness.logger, event(with_id(inference(), "call-a"))).await;
        failure(
            &harness.logger,
            event(with_id(inference_failure(), "call-a")),
        )
        .await;
        assert_eq!(finished(&harness.spans).len(), 1);
    }

    #[tokio::test]
    async fn llm_close_emits_deferred_when_a_payload_proves_the_call_ran() {
        let harness = harness(owned_http(), anchored());
        success(&harness.logger, event(with_id(inference(), "call-a"))).await;
        let spans = finished(&harness.spans);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans.first().expect("span").start_time, payload_start());
    }

    #[tokio::test]
    async fn llm_close_skips_a_synthetic_no_upstream_call_without_a_carrier() {
        let harness = harness(owned_http(), anchored());
        failure(
            &harness.logger,
            event_with(
                with_id(inference_failure(), "call-a"),
                Json::Null,
                no_upstream(),
            ),
        )
        .await;
        assert!(finished(&harness.spans).is_empty());
    }

    #[tokio::test]
    async fn llm_close_skips_a_call_that_never_reached_the_provider() {
        let harness = harness(owned_http(), anchored());
        success(
            &harness.logger,
            event(with_id(zero_start(inference()), "call-a")),
        )
        .await;
        assert!(finished(&harness.spans).is_empty());
    }
}

mod mcp_list {
    use opentelemetry::trace::SpanId;

    use crate::otel::support::{
        anchored, event, failure, finished, harness, harness_for, mcp_list, mcp_list_failure,
        mcp_list_with_auth, names, owned_http, phoenix_http, root_trace_id, success, with_id,
    };

    #[tokio::test]
    async fn mcp_list_emits_one_whole_span_for_the_call() {
        let harness = harness(owned_http(), anchored());
        success(&harness.logger, event(with_id(mcp_list(), "call-list"))).await;
        let spans = finished(&harness.spans);
        assert_eq!(spans.len(), 1);
        assert_eq!(
            names(&spans).first().map(String::as_str),
            Some("tools/list")
        );
    }

    #[tokio::test]
    async fn mcp_list_dedups_the_success_and_failure_emissions() {
        let harness = harness(owned_http(), anchored());
        success(&harness.logger, event(with_id(mcp_list(), "call-list"))).await;
        failure(
            &harness.logger,
            event(with_id(mcp_list_failure(), "call-list")),
        )
        .await;
        assert_eq!(finished(&harness.spans).len(), 1);
    }

    #[tokio::test]
    async fn mcp_list_roots_a_detached_span_and_links_the_request_trace() {
        let harness = harness_for(phoenix_http(), anchored(), "arize_phoenix", None);
        success(
            &harness.logger,
            event(with_id(mcp_list_with_auth(), "call-list")),
        )
        .await;
        let spans = finished(&harness.built.sink(0));
        assert_eq!(spans.len(), 1);
        let span = spans.first().expect("span");
        assert_eq!(span.parent_span_id, SpanId::INVALID);
        assert!(
            span.links
                .iter()
                .any(|link| link.span_context.trace_id() == root_trace_id())
        );
        let specs = harness.built.specs();
        let headers = specs
            .first()
            .and_then(|spec| spec.headers.clone())
            .unwrap_or_default();
        assert!(headers.contains("x-project-name=proj"));
    }
}

mod mcp_tool {
    use opentelemetry::trace::SpanId;

    use crate::otel::support::{
        anchored, event, failure, finished, harness, harness_for, inference, mcp_tool,
        mcp_tool_failure, mcp_tool_with_auth, names, owned_http, payload_start, phoenix_http,
        pre_call, root_trace_id, success, with_id,
    };

    #[tokio::test]
    async fn mcp_tool_emits_one_whole_span_for_the_call() {
        let harness = harness(owned_http(), anchored());
        success(&harness.logger, event(with_id(mcp_tool(), "call-mcp"))).await;
        let spans = finished(&harness.spans);
        assert_eq!(spans.len(), 1);
        assert_eq!(
            names(&spans).first().map(String::as_str),
            Some("tools/call search")
        );
    }

    #[tokio::test]
    async fn mcp_tool_emission_drops_the_stray_llm_carrier() {
        let harness = harness(owned_http(), anchored());
        pre_call(&harness.logger, "call-mcp", "gpt-4o", false, None, None);
        success(&harness.logger, event(with_id(mcp_tool(), "call-mcp"))).await;
        success(&harness.logger, event(with_id(inference(), "call-mcp"))).await;
        let spans = finished(&harness.spans);
        assert_eq!(spans.len(), 2);
        assert_eq!(
            names(&spans).first().map(String::as_str),
            Some("tools/call search")
        );
        // the stray carrier is gone, so the later close emits deferred from the payload's own time
        assert_eq!(spans.get(1).expect("llm span").start_time, payload_start());
    }

    #[tokio::test]
    async fn mcp_tool_dedups_the_success_and_failure_emissions() {
        let harness = harness(owned_http(), anchored());
        success(&harness.logger, event(with_id(mcp_tool(), "call-mcp"))).await;
        failure(
            &harness.logger,
            event(with_id(mcp_tool_failure(), "call-mcp")),
        )
        .await;
        assert_eq!(finished(&harness.spans).len(), 1);
    }

    #[tokio::test]
    async fn mcp_tool_roots_a_detached_span_and_links_the_request_trace() {
        let harness = harness_for(phoenix_http(), anchored(), "arize_phoenix", None);
        success(
            &harness.logger,
            event(with_id(mcp_tool_with_auth(), "call-mcp")),
        )
        .await;
        let spans = finished(&harness.built.sink(0));
        assert_eq!(spans.len(), 1);
        let span = spans.first().expect("span");
        assert_eq!(span.parent_span_id, SpanId::INVALID);
        assert!(
            span.links
                .iter()
                .any(|link| link.span_context.trace_id() == root_trace_id())
        );
    }
}

mod metrics {
    use betterlite_logging::otel::model::utils::coercion::OtelModelValue;
    use betterlite_models::decisions::Json;
    use tracing_subscriber::layer::SubscriberExt as _;

    use crate::otel::support::{
        Captured, anchored, event, event_with, failure, harness_with, inference, inference_failure,
        metric_points, no_upstream, owned_http, recorder, success, with_id,
    };

    #[tokio::test]
    async fn metrics_record_a_successful_call() {
        let (recorder, provider, exporter) = recorder(None);
        let harness = harness_with(owned_http(), anchored(), Some(recorder));
        success(&harness.logger, event(with_id(inference(), "call-a"))).await;
        assert_ne!(metric_points(&provider, &exporter), 0);
    }

    #[tokio::test]
    async fn metrics_record_a_failed_call() {
        let (recorder, provider, exporter) = recorder(None);
        let harness = harness_with(owned_http(), anchored(), Some(recorder));
        failure(
            &harness.logger,
            event(with_id(inference_failure(), "call-b")),
        )
        .await;
        assert_ne!(metric_points(&provider, &exporter), 0);
    }

    #[tokio::test]
    async fn metrics_skip_a_synthetic_no_upstream_failure() {
        let (recorder, provider, exporter) = recorder(None);
        let harness = harness_with(owned_http(), anchored(), Some(recorder));
        failure(
            &harness.logger,
            event_with(
                with_id(inference_failure(), "call-c"),
                Json::Null,
                no_upstream(),
            ),
        )
        .await;
        assert_eq!(metric_points(&provider, &exporter), 0);
    }

    #[tokio::test]
    async fn metrics_log_a_filter_error_once_and_then_stay_silent() {
        let (recorder, _, _) = recorder(Some(OtelModelValue::String("not a mapping".to_owned())));
        let harness = harness_with(owned_http(), anchored(), Some(recorder));
        let captured = Captured::default();
        let subscriber = tracing_subscriber::registry().with(captured.clone());
        let _guard = tracing::subscriber::set_default(subscriber);
        success(&harness.logger, event(with_id(inference(), "call-a"))).await;
        success(&harness.logger, event(with_id(inference(), "call-b"))).await;
        let messages = captured.messages();
        assert_eq!(messages.len(), 1);
        assert!(
            messages
                .first()
                .expect("message")
                .contains("invalid otel.attributes metric filter")
        );
    }
}

mod open_calls {
    use std::time::{Duration, SystemTime};

    use betterlite_logging::otel::logger::v2::{OPEN_CALLS_MAX, OpenCall, OpenCalls};
    use indexmap::IndexMap;

    use crate::otel::support::{
        anchored, creds, event_with, finished, harness, inference, owned_http, payload_start,
        pre_call, success, with_id,
    };

    #[test]
    fn open_calls_caps_the_map_and_evicts_the_oldest() {
        let mut calls = OpenCalls::default();
        let start = SystemTime::now();
        for index in 0..OPEN_CALLS_MAX {
            let at = start
                .checked_add(Duration::from_secs(u64::try_from(index).expect("index")))
                .expect("carrier time");
            assert!(
                calls
                    .store(
                        format!("call-{index}"),
                        OpenCall {
                            span: None,
                            start_time: at,
                            provider: None,
                        },
                    )
                    .is_none()
            );
        }
        let evicted = calls
            .store(
                "overflow".to_owned(),
                OpenCall {
                    span: None,
                    start_time: start,
                    provider: None,
                },
            )
            .expect("evicted carrier");
        assert_eq!(evicted.start_time, start);
        assert!(!calls.contains("call-0"));
        assert!(calls.contains("overflow"));
        // pop is the dedup between the success and failure paths
        assert!(calls.pop(Some("call-1")).is_some());
        assert!(calls.pop(Some("call-1")).is_none());
    }

    #[tokio::test]
    async fn overflow_eviction_still_emits_the_evicted_call_deferred() {
        let harness = harness(owned_http(), anchored());
        for index in 0..=OPEN_CALLS_MAX {
            pre_call(
                &harness.logger,
                &format!("call-{index}"),
                "gpt-4o",
                false,
                Some(creds("S")),
                None,
            );
        }
        assert_eq!(harness.built.builds(), 1);
        success(
            &harness.logger,
            event_with(with_id(inference(), "call-0"), creds("S"), IndexMap::new()),
        )
        .await;
        success(
            &harness.logger,
            event_with(with_id(inference(), "call-1"), creds("S"), IndexMap::new()),
        )
        .await;
        let spans = finished(&harness.built.sink(0));
        assert_eq!(spans.len(), 2);
        // call-0 lost its carrier: its span is created at close from the payload's own start
        assert_eq!(
            spans.first().expect("deferred span").start_time,
            payload_start()
        );
        // call-1 kept its carrier: its span was born at the pre_call boundary
        assert_ne!(spans.get(1).expect("live span").start_time, payload_start());
    }
}

mod pre_call {
    use betterlite_logging::logging_pre_post::hooks::PreCallEvent;
    use betterlite_logging::otel::logger::pre_call::log_pre_api_call;
    use betterlite_models::decisions::Json;
    use indexmap::IndexMap;
    use serde_json::json;

    use crate::otel::support::{
        anchored, creds, empty_trace, event, event_with, finished, harness, inference, owned_http,
        payload_start, pre_call, success, with_id,
    };

    #[tokio::test]
    async fn pre_call_opens_the_span_and_holds_the_route_at_the_boundary() {
        let harness = harness(owned_http(), anchored());
        pre_call(
            &harness.logger,
            "call-a",
            "gpt-4o",
            false,
            Some(creds("S")),
            None,
        );
        assert_eq!(harness.built.builds(), 1);
        success(&harness.logger, event(with_id(inference(), "call-a"))).await;
        // the live span closes without routing again
        assert_eq!(harness.built.builds(), 1);
        let spans = finished(&harness.built.sink(0));
        assert_eq!(spans.len(), 1);
        assert_ne!(spans.first().expect("span").start_time, payload_start());
    }

    #[tokio::test]
    async fn pre_call_defers_the_span_and_routes_at_close_without_a_parent() {
        let harness = harness(owned_http(), empty_trace());
        pre_call(
            &harness.logger,
            "call-a",
            "gpt-4o",
            false,
            Some(creds("S")),
            None,
        );
        assert_eq!(harness.built.builds(), 0);
        success(
            &harness.logger,
            event_with(with_id(inference(), "call-a"), creds("S"), IndexMap::new()),
        )
        .await;
        assert_eq!(harness.built.builds(), 1);
        assert_eq!(finished(&harness.built.sink(0)).len(), 1);
    }

    #[tokio::test]
    async fn pre_call_skips_a_synthetic_no_upstream_call() {
        let harness = harness(owned_http(), anchored());
        pre_call(
            &harness.logger,
            "call-a",
            "gpt-4o",
            true,
            Some(creds("S")),
            None,
        );
        assert_eq!(harness.built.builds(), 0);
        // no carrier was stored, so a real boundary for the same id still opens one
        pre_call(
            &harness.logger,
            "call-a",
            "gpt-4o",
            false,
            Some(creds("S")),
            None,
        );
        assert_eq!(harness.built.builds(), 1);
    }

    #[tokio::test]
    async fn pre_call_keeps_the_first_carrier_for_a_repeated_call_id() {
        let harness = harness(owned_http(), anchored());
        pre_call(
            &harness.logger,
            "call-a",
            "gpt-4o",
            false,
            Some(creds("S1")),
            None,
        );
        pre_call(
            &harness.logger,
            "call-a",
            "gpt-4o",
            false,
            Some(creds("S2")),
            None,
        );
        assert_eq!(harness.built.builds(), 1);
        let specs = harness.built.specs();
        assert_eq!(specs.len(), 1);
        let headers = specs
            .first()
            .and_then(|spec| spec.headers.clone())
            .unwrap_or_default();
        assert!(headers.contains("arize-space-id=S1"));
        assert!(!headers.contains("S2"));
    }

    #[tokio::test]
    async fn pre_call_ignores_a_boundary_without_a_call_id() {
        let harness = harness(owned_http(), anchored());
        let details: Json = serde_json::from_value(json!({
            "model": "gpt-4o",
            "call_type": "completion",
            "standard_callback_dynamic_params": {"arize_space_id": "S", "arize_api_key": "K"},
        }))
        .expect("boundary details");
        let messages = Json::List(Vec::new());
        log_pre_api_call(
            &harness.logger,
            &PreCallEvent {
                model: "gpt-4o",
                messages: &messages,
                details: &details,
            },
        )
        .expect("pre api call");
        assert_eq!(harness.built.builds(), 0);
    }
}
