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
        Arc::new(support::FixedIds),
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
    let _event = logger.log_event_on_langfuse(&call).await;
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
    let _event = logger.log_event_on_langfuse(&call).await;
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
    let _event = logger.log_event_on_langfuse(&call).await;
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
