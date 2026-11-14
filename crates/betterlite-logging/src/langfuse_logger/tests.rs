#![cfg(test)]

pub(crate) mod support;

mod initialization_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use crate::langfuse_channels::{CHANNEL_RETIRE_GRACE, release_langfuse_tracing};
    use crate::langfuse_logger::LangfuseLoggerError;
    use crate::langfuse_logger::budget::LangfuseClientBudget;
    use crate::langfuse_logger::langfuse::LangFuseLogger;
    use crate::langfuse_rest::client::LangfuseClientConfig;

    use super::support;

    #[tokio::test]
    async fn environment_credentials_build_a_logger() {
        let services = support::services(
            &[
                ("LANGFUSE_PUBLIC_KEY", "pk-init"),
                ("LANGFUSE_SECRET_KEY", "sk-init"),
                ("LANGFUSE_MOCK", "true"),
            ],
            10,
        );
        let (log, guard) = support::capture_log();
        let logger = LangFuseLogger::new(&services, &support::environment_request())
            .await
            .expect("the logger builds");
        assert_eq!(logger.project_id.as_deref(), Some("mock-project-id"));
        let output = log.contents();
        assert!(output.contains("Langfuse Mock: Using mock project ID"));
        assert!(output.contains("Created langfuse client number 1"));
        drop(guard);
    }

    #[tokio::test]
    async fn an_unusable_prompt_cache_ttl_fails_before_the_client() {
        let services = support::services(
            &[
                ("LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS", "thirty"),
                ("LANGFUSE_MOCK", "true"),
            ],
            10,
        );
        let error = LangFuseLogger::new(&services, &support::environment_request())
            .await
            .err()
            .expect("the prompt cache ttl is refused");
        assert!(matches!(error, LangfuseLoggerError::Settings(_)));
        assert!(
            error
                .to_string()
                .contains("LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS='thirty'")
        );
    }

    #[tokio::test]
    async fn a_project_id_lookup_failure_only_logs() {
        let services = support::services(
            &[
                ("LANGFUSE_PUBLIC_KEY", "pk-project"),
                ("LANGFUSE_SECRET_KEY", "sk-project"),
            ],
            10,
        );
        let (log, guard) = support::capture_log();
        let logger = LangFuseLogger::new(&services, &support::environment_request())
            .await
            .expect("the logger builds without a project id");
        assert_eq!(logger.project_id, None);
        assert!(log.contents().contains("Langfuse project id unavailable"));
        drop(guard);
    }

    #[tokio::test]
    async fn an_invalid_explicit_environment_is_refused() {
        let services = support::services(&[("LANGFUSE_MOCK", "true")], 10);
        let mut request = support::environment_request();
        request.environment = Some("Invalid Env");
        let error = LangFuseLogger::new(&services, &request)
            .await
            .err()
            .expect("the environment is refused");
        assert!(matches!(error, LangfuseLoggerError::Settings(_)));
        assert!(
            error
                .to_string()
                .contains("invalid callback field langfuse_environment")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_client_build_failure_gives_the_channel_back() {
        let services = support::services_with(
            &[],
            LangfuseClientBudget::new(1),
            Arc::new(support::FailingBuilder),
            None,
        );
        let request = support::request("pk-failing", "sk-failing");
        let (exporter, recording) = support::recording();
        let probe = support::seed(&services, &request, exporter);
        release_langfuse_tracing(&probe, CHANNEL_RETIRE_GRACE).await;
        let error = LangFuseLogger::new(&services, &request)
            .await
            .err()
            .expect("the client build fails");
        assert!(matches!(error, LangfuseLoggerError::Client(_)));
        assert_eq!(recording.shut_down(), 1);
        assert!(services.budget.reserve().is_ok());
    }

    #[tokio::test]
    async fn an_unusable_timeout_is_named() {
        let services =
            support::services(&[("LANGFUSE_TIMEOUT", "-1"), ("LANGFUSE_MOCK", "true")], 10);
        let error = LangFuseLogger::new(&services, &support::environment_request())
            .await
            .err()
            .expect("the timeout is refused");
        assert!(matches!(error, LangfuseLoggerError::Timeout { .. }));
        assert_eq!(
            error.to_string(),
            "LANGFUSE_TIMEOUT='-1' is not a number of seconds above 0"
        );
    }

    #[test]
    fn the_http_builder_builds_from_the_injected_client() {
        let builder = support::real_builder();
        let config = LangfuseClientConfig {
            public_key: Some("pk-http".to_owned()),
            secret_key: Some(support::secret("sk-http")),
            base_url: support::HOST.to_owned(),
            timeout: Duration::from_secs(20),
            prompt_cache_ttl: Duration::from_secs(60),
        };
        assert!(builder.build(config).is_ok());
    }

    #[tokio::test]
    async fn initialization_warns_about_the_removed_upstream_client() {
        let services = support::services(
            &[
                ("LANGFUSE_MOCK", "true"),
                ("UPSTREAM_LANGFUSE_SECRET_KEY", "sk-upstream"),
            ],
            10,
        );
        let (log, guard) = support::capture_log();
        LangFuseLogger::new(&services, &support::environment_request())
            .await
            .expect("the logger builds");
        assert!(
            log.contents()
                .contains("UPSTREAM_LANGFUSE_* is no longer supported")
        );
        drop(guard);
    }
}

mod budget_enforcement {
    use std::sync::Arc;

    use crate::langfuse_logger::LangfuseLoggerError;
    use crate::langfuse_logger::budget::{
        CLIENT_LIMIT_VARIABLE, DEFAULT_CLIENT_LIMIT, LangfuseBudgetError, LangfuseClientBudget,
    };
    use crate::langfuse_logger::langfuse::LangFuseLogger;

    use super::support;

    #[test]
    fn the_limit_comes_from_the_environment() {
        let unset = LangfuseClientBudget::from_environment(&support::lookup(&[]))
            .expect("the default limit");
        assert_eq!(unset.limit, DEFAULT_CLIENT_LIMIT);
        let empty = LangfuseClientBudget::from_environment(&support::lookup(&[(
            CLIENT_LIMIT_VARIABLE,
            "",
        )]))
        .expect("the default limit");
        assert_eq!(empty.limit, DEFAULT_CLIENT_LIMIT);
        let configured = LangfuseClientBudget::from_environment(&support::lookup(&[(
            CLIENT_LIMIT_VARIABLE,
            "3",
        )]))
        .expect("the configured limit");
        assert_eq!(configured.limit, 3);
        let refused = LangfuseClientBudget::from_environment(&support::lookup(&[(
            CLIENT_LIMIT_VARIABLE,
            "many",
        )]))
        .err()
        .expect("a non-number is refused");
        assert!(matches!(refused, LangfuseBudgetError::Limit(_)));
    }

    #[tokio::test]
    async fn a_full_budget_refuses_another_logger() {
        let services = support::services(&[("LANGFUSE_MOCK", "true")], 1);
        let _first =
            LangFuseLogger::new(&services, &support::request("pk-budget-1", "sk-budget-1"))
                .await
                .expect("the first logger builds");
        let error = LangFuseLogger::new(&services, &support::request("pk-budget-2", "sk-budget-2"))
            .await
            .err()
            .expect("the budget refuses the second");
        assert!(matches!(error, LangfuseLoggerError::Budget(_)));
        assert_eq!(
            error.to_string(),
            "Max langfuse clients reached: 1 is greater than 1"
        );
    }

    #[tokio::test]
    async fn a_failed_client_build_does_not_hold_a_slot() {
        let budget = LangfuseClientBudget::new(1);
        let failing =
            support::services_with(&[], budget.clone(), Arc::new(support::FailingBuilder), None);
        let error = LangFuseLogger::new(&failing, &support::request("pk-slot", "sk-slot"))
            .await
            .err()
            .expect("the client build fails");
        assert!(matches!(error, LangfuseLoggerError::Client(_)));
        let mock = support::services_with(
            &[("LANGFUSE_MOCK", "true")],
            budget,
            support::real_builder(),
            None,
        );
        LangFuseLogger::new(&mock, &support::request("pk-slot-2", "sk-slot-2"))
            .await
            .expect("the slot was given back");
    }

    #[tokio::test]
    async fn stop_returns_the_slot() {
        let services = support::services(&[("LANGFUSE_MOCK", "true")], 1);
        let logger = LangFuseLogger::new(&services, &support::request("pk-stop", "sk-stop"))
            .await
            .expect("the logger builds");
        logger.stop();
        LangFuseLogger::new(&services, &support::request("pk-stop-2", "sk-stop-2"))
            .await
            .expect("the slot was given back");
    }
}

mod lifecycle_tests {
    use std::time::Duration;

    use tokio::task::yield_now;
    use tokio::time::advance;

    use crate::langfuse_channels::{CHANNEL_RETIRE_GRACE, release_langfuse_tracing};
    use crate::langfuse_logger::LangfuseLoggerError;
    use crate::langfuse_logger::langfuse::{LangFuseLogger, LangfuseLoggedEvent};

    use super::support;

    #[tokio::test]
    async fn flush_drains_the_channel() {
        let services = support::services(&[], 10);
        let request = support::request("pk-flush", "sk-flush");
        let (exporter, recording) = support::recording();
        let _probe = support::seed(&services, &request, exporter);
        let logger = LangFuseLogger::new(&services, &request)
            .await
            .expect("the logger builds");
        let payload = support::payload(support::TRACE_ID);
        let response = support::response();
        let params = support::optional_params();
        let litellm_params = support::litellm_params(false);
        let call = support::call(
            &payload,
            &response,
            &params,
            &litellm_params,
            support::start(),
        );
        let event = logger.log_event_on_langfuse(&call).await;
        assert_eq!(event.trace_id.as_deref(), Some(support::TRACE_ID));
        assert!(recording.spans().is_empty());
        assert!(logger.flush().await);
        assert!(!recording.spans().is_empty());
    }

    #[tokio::test]
    async fn a_failed_export_makes_the_flush_report_failure() {
        let services = support::services(&[], 10);
        let request = support::request("pk-flush-fail", "sk-flush-fail");
        let _probe = support::seed(&services, &request, support::Rejecting);
        let logger = LangFuseLogger::new(&services, &request)
            .await
            .expect("the logger builds");
        let payload = support::payload(support::TRACE_ID);
        let response = support::response();
        let params = support::optional_params();
        let litellm_params = support::litellm_params(false);
        let call = support::call(
            &payload,
            &response,
            &params,
            &litellm_params,
            support::start(),
        );
        let (log, guard) = support::capture_log();
        logger.log_event_on_langfuse(&call).await;
        assert!(!logger.flush().await);
        assert!(
            log.contents()
                .contains("the langfuse export channel did not drain before the flush deadline")
        );
        drop(guard);
    }

    #[tokio::test(start_paused = true)]
    async fn stop_keeps_the_channel_for_the_grace_period() {
        let services = support::services(&[], 10);
        let request = support::request("pk-grace", "sk-grace");
        let (exporter, _recording) = support::recording();
        let probe = support::seed(&services, &request, exporter);
        let logger = LangFuseLogger::new(&services, &request)
            .await
            .expect("the logger builds");
        release_langfuse_tracing(&probe, CHANNEL_RETIRE_GRACE).await;
        logger.stop();
        yield_now().await;
        let (exporter, rebuilt) = support::recording();
        let _reseeded = support::seed(&services, &request, exporter);
        assert_eq!(rebuilt.installed(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn stop_retires_the_channel_after_the_grace_period() {
        let services = support::services(&[], 10);
        let request = support::request("pk-retire", "sk-retire");
        let (exporter, _recording) = support::recording();
        let probe = support::seed(&services, &request, exporter);
        let logger = LangFuseLogger::new(&services, &request)
            .await
            .expect("the logger builds");
        release_langfuse_tracing(&probe, CHANNEL_RETIRE_GRACE).await;
        logger.stop();
        yield_now().await;
        yield_now().await;
        advance(Duration::from_secs(60)).await;
        yield_now().await;
        let (exporter, rebuilt) = support::recording();
        let _reseeded = support::seed(&services, &request, exporter);
        assert_eq!(rebuilt.installed(), 1);
    }

    #[tokio::test]
    async fn stop_is_idempotent() {
        let services = support::services(&[("LANGFUSE_MOCK", "true")], 2);
        let first = LangFuseLogger::new(&services, &support::request("pk-idem-1", "sk-idem-1"))
            .await
            .expect("the first logger builds");
        let _second = LangFuseLogger::new(&services, &support::request("pk-idem-2", "sk-idem-2"))
            .await
            .expect("the second logger builds");
        first.stop();
        first.stop();
        LangFuseLogger::new(&services, &support::request("pk-idem-3", "sk-idem-3"))
            .await
            .expect("the released slot is free");
        let error = LangFuseLogger::new(&services, &support::request("pk-idem-4", "sk-idem-4"))
            .await
            .err()
            .expect("the budget stays full");
        assert!(matches!(error, LangfuseLoggerError::Budget(_)));
    }

    #[tokio::test]
    async fn flush_and_stop_are_callable_after_a_failed_event() {
        let services = support::services(&[], 10);
        let request = support::request("pk-life-fail", "sk-life-fail");
        let (exporter, _recording) = support::recording();
        let _probe = support::seed(&services, &request, exporter);
        let logger = LangFuseLogger::new(&services, &request)
            .await
            .expect("the logger builds");
        let payload = support::payload(support::TRACE_ID);
        let response = support::failed_response();
        let params = support::optional_params();
        let litellm_params = support::litellm_params(false);
        let call = support::call(
            &payload,
            &response,
            &params,
            &litellm_params,
            support::start(),
        );
        let event = logger.log_event_on_langfuse(&call).await;
        assert_eq!(event, LangfuseLoggedEvent::default());
        assert!(logger.flush().await);
        logger.stop();
    }
}

mod event_dispatch_tests {
    use std::sync::Arc;

    use betterlite_models::decisions::Json;

    use crate::langfuse_logger::budget::LangfuseClientBudget;
    use crate::langfuse_logger::langfuse::{LangFuseLogger, LangfuseLoggedEvent};

    use super::support;

    #[tokio::test]
    async fn an_event_returns_the_trace_and_generation_ids() {
        let services = support::services(&[], 10);
        let request = support::request("pk-event", "sk-event");
        let (exporter, _recording) = support::recording();
        let _probe = support::seed(&services, &request, exporter);
        let logger = LangFuseLogger::new(&services, &request)
            .await
            .expect("the logger builds");
        let payload = support::payload(support::TRACE_ID);
        let response = support::response();
        let params = support::optional_params();
        let litellm_params = support::litellm_params(false);
        let call = support::call(
            &payload,
            &response,
            &params,
            &litellm_params,
            support::start(),
        );
        let event = logger.log_event_on_langfuse(&call).await;
        assert_eq!(event.trace_id.as_deref(), Some(support::TRACE_ID));
        assert_eq!(event.generation_id.as_deref(), Some(support::GENERATION_ID));
    }

    #[tokio::test]
    async fn an_empty_trace_id_is_drawn_from_the_injected_generator() {
        let services = support::services_with(
            &[],
            LangfuseClientBudget::new(10),
            support::real_builder(),
            Some(Arc::new(support::FixedIds)),
        );
        let request = support::request("pk-fixed", "sk-fixed");
        let (exporter, _recording) = support::recording();
        let _probe = support::seed(&services, &request, exporter);
        let logger = LangFuseLogger::new(&services, &request)
            .await
            .expect("the logger builds");
        let payload = support::payload("");
        let response = support::response();
        let params = support::optional_params();
        let litellm_params = support::litellm_params(false);
        let call = support::call(
            &payload,
            &response,
            &params,
            &litellm_params,
            support::start(),
        );
        let event = logger.log_event_on_langfuse(&call).await;
        assert_eq!(event.trace_id.as_deref(), Some(support::FIXED_TRACE_ID));
        assert_eq!(event.generation_id.as_deref(), Some(support::GENERATION_ID));
    }

    #[tokio::test]
    async fn the_span_carries_header_metadata_and_input() {
        let services = support::services(&[], 10);
        let request = support::request("pk-span-input", "sk-span-input");
        let (exporter, recording) = support::recording();
        let _probe = support::seed(&services, &request, exporter);
        let logger = LangFuseLogger::new(&services, &request)
            .await
            .expect("the logger builds");
        let payload = support::payload(support::TRACE_ID);
        let response = support::response();
        let params = support::optional_params();
        let litellm_params = support::litellm_params(false);
        let call = support::call(
            &payload,
            &response,
            &params,
            &litellm_params,
            support::start(),
        );
        logger.log_event_on_langfuse(&call).await;
        assert!(logger.flush().await);
        let spans = recording.spans();
        assert_eq!(
            support::attribute_text(&spans, "langfuse.trace.name").as_deref(),
            Some("from-header")
        );
        let input = support::object(support::attribute_json(
            &spans,
            "langfuse.observation.input",
        ));
        assert_eq!(input.get("messages"), payload.messages.as_ref());
        assert!(input.contains_key("tools"));
        assert!(!input.contains_key("functions"));
        assert!(!input.contains_key("secret_fields"));
    }

    #[tokio::test]
    async fn the_span_carries_sanitized_parameters() {
        let services = support::services(&[], 10);
        let request = support::request("pk-span-params", "sk-span-params");
        let (exporter, recording) = support::recording();
        let _probe = support::seed(&services, &request, exporter);
        let logger = LangFuseLogger::new(&services, &request)
            .await
            .expect("the logger builds");
        let payload = support::payload(support::TRACE_ID);
        let response = support::response();
        let params = support::optional_params();
        let litellm_params = support::litellm_params(false);
        let call = support::call(
            &payload,
            &response,
            &params,
            &litellm_params,
            support::start(),
        );
        logger.log_event_on_langfuse(&call).await;
        assert!(logger.flush().await);
        let spans = recording.spans();
        let parameters = support::object(support::attribute_json(
            &spans,
            "langfuse.observation.model.parameters",
        ));
        assert!(parameters.contains_key("temperature"));
        assert!(!parameters.contains_key("secret_fields"));
        assert_eq!(
            parameters.get("custom"),
            Some(&Json::Text(r#"{"nested":1}"#.to_owned()))
        );
    }

    #[tokio::test]
    async fn the_span_carries_the_litellm_params_metadata_in_debug_mode() {
        let services = support::services(&[], 10);
        let request = support::request("pk-span-debug", "sk-span-debug");
        let (exporter, recording) = support::recording();
        let _probe = support::seed(&services, &request, exporter);
        let logger = LangFuseLogger::new(&services, &request)
            .await
            .expect("the logger builds");
        let payload = support::payload(support::TRACE_ID);
        let response = support::response();
        let params = support::optional_params();
        let litellm_params = support::litellm_params(true);
        let call = support::call(
            &payload,
            &response,
            &params,
            &litellm_params,
            support::start(),
        );
        logger.log_event_on_langfuse(&call).await;
        assert!(logger.flush().await);
        let spans = recording.spans();
        let passed =
            support::attribute_text(&spans, "langfuse.trace.metadata.metadata_passed_to_litellm")
                .expect("the debug metadata carries the litellm_params metadata");
        assert!(passed.contains(r#""user_api_key_alias":"alias-1""#));
    }

    #[tokio::test]
    async fn a_failing_assembly_returns_empty_ids() {
        let services = support::services(&[], 10);
        let request = support::request("pk-event-fail", "sk-event-fail");
        let (exporter, _recording) = support::recording();
        let _probe = support::seed(&services, &request, exporter);
        let logger = LangFuseLogger::new(&services, &request)
            .await
            .expect("the logger builds");
        let payload = support::payload(support::TRACE_ID);
        let response = support::failed_response();
        let params = support::optional_params();
        let litellm_params = support::litellm_params(false);
        let call = support::call(
            &payload,
            &response,
            &params,
            &litellm_params,
            support::start(),
        );
        let event = logger.log_event_on_langfuse(&call).await;
        assert_eq!(event, LangfuseLoggedEvent::default());
    }
}
