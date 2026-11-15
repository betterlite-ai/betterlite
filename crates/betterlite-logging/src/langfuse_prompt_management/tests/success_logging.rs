use std::sync::Arc;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tokio::task::yield_now;
use tokio::time::advance;

use super::support;
use crate::callback_logger::CallbackLogger;
use crate::langfuse_channels::{CHANNEL_RETIRE_GRACE, release_langfuse_tracing};
use crate::langfuse_prompt_management::LangfusePromptManagement;

#[tokio::test]
async fn the_success_span_carries_the_messages_and_the_response() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    let payload = support::success_payload(Some(support::CALL_ID));
    let messages = payload.messages.clone();
    fixture
        .manager
        .async_log_success_event(support::event(payload))
        .await
        .expect("the event logs");
    assert!(fixture.manager.logger.flush().await);
    let spans = fixture.spans.spans();
    assert_eq!(
        support::langfuse::attribute_text(&spans, "langfuse.observation.level").as_deref(),
        Some("DEFAULT")
    );
    let input = support::langfuse::object(support::langfuse::attribute_json(
        &spans,
        "langfuse.observation.input",
    ));
    assert_eq!(input.get("messages"), messages.as_ref());
    let output = support::langfuse::attribute_text(&spans, "langfuse.observation.output")
        .expect("the response is projected as the output");
    assert!(output.contains("hi"), "the output was {output}");
}

#[tokio::test]
async fn the_success_span_writes_completion_start_as_rfc3339_text() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    fixture
        .manager
        .async_log_success_event(support::event(support::success_payload(None)))
        .await
        .expect("the event logs");
    assert!(fixture.manager.logger.flush().await);
    let spans = fixture.spans.spans();
    assert_eq!(
        support::langfuse::attribute_text(&spans, "langfuse.observation.completion_start_time")
            .as_deref(),
        Some("\"2024-01-01T00:00:00.500+00:00\"")
    );
}

#[tokio::test]
async fn the_success_span_takes_its_trace_name_from_the_request_metadata() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    let metadata = IndexMap::from([("trace_name".to_owned(), Json::Text("acme".to_owned()))]);
    let event = support::event_with(
        support::success_payload(Some(support::CALL_ID)),
        support::no_dynamic_params(),
        metadata,
    );
    fixture
        .manager
        .async_log_success_event(event)
        .await
        .expect("the event logs");
    assert!(fixture.manager.logger.flush().await);
    let spans = fixture.spans.spans();
    assert_eq!(
        support::langfuse::attribute_text(&spans, "langfuse.trace.name").as_deref(),
        Some("acme")
    );
}

#[tokio::test]
async fn a_request_with_dynamic_credentials_logs_through_its_own_client() {
    let builder = support::RecordingBuilder::default();
    let variables = support::env(support::HOST);
    let services = support::services(&support::borrowed(&variables), Arc::new(builder.clone()));
    let (exporter, _spans) = support::langfuse::recording();
    let _probe = support::langfuse::seed(
        &services,
        &support::langfuse::environment_request(),
        exporter,
    );
    let manager = LangfusePromptManagement::new(services, support::trace_cache())
        .await
        .expect("the manager builds");
    let dynamic = support::dynamic_params(&[
        ("langfuse_host", "http://127.0.0.1:2"),
        ("langfuse_public_key", "pk-dynamic"),
        ("langfuse_secret", "sk-dynamic"),
    ]);
    let event = support::event_with(
        support::success_payload(Some(support::CALL_ID)),
        dynamic,
        IndexMap::new(),
    );
    manager
        .async_log_success_event(event)
        .await
        .expect("the event logs");
    let configs = builder.configs();
    assert_eq!(configs.len(), 2);
    assert_eq!(configs[1].public_key.as_deref(), Some("pk-dynamic"));
    assert_eq!(configs[1].secret.as_deref(), Some("sk-dynamic"));
    assert_eq!(configs[1].base_url, "http://127.0.0.1:2");
    let trace = manager
        .trace_cache
        .get_cache(support::CALL_ID, "langfuse")
        .expect("the trace cache reads");
    assert_eq!(trace.as_deref(), Some(support::TRACE_ID));
}

#[tokio::test(start_paused = true)]
async fn a_dynamic_credential_event_stops_the_logger_it_built() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    let services = support::services(
        &support::borrowed(&variables),
        support::langfuse::real_builder(),
    );
    let request =
        support::dynamic_request("pk-dynamic-stop", "sk-dynamic-stop", "http://127.0.0.1:5");
    let (exporter, _spans) = support::langfuse::recording();
    let probe = support::langfuse::seed(&services, &request, exporter);
    let dynamic = support::dynamic_params(&[
        ("langfuse_host", "http://127.0.0.1:5"),
        ("langfuse_public_key", "pk-dynamic-stop"),
        ("langfuse_secret", "sk-dynamic-stop"),
    ]);
    let event = support::event_with(
        support::success_payload(Some(support::CALL_ID)),
        dynamic,
        IndexMap::new(),
    );
    fixture
        .manager
        .async_log_success_event(event)
        .await
        .expect("the event logs");
    release_langfuse_tracing(&probe, CHANNEL_RETIRE_GRACE).await;
    yield_now().await;
    yield_now().await;
    advance(CHANNEL_RETIRE_GRACE).await;
    yield_now().await;
    let (rebuilt_exporter, rebuilt) = support::langfuse::recording();
    let _reseeded = support::langfuse::seed(&services, &request, rebuilt_exporter);
    assert_eq!(rebuilt.installed(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_success_event_keeps_the_managers_own_logger_running() {
    let variables = support::env("http://127.0.0.1:3");
    let fixture = support::manager(&support::borrowed(&variables)).await;
    fixture
        .manager
        .async_log_success_event(support::event(support::success_payload(None)))
        .await
        .expect("the event logs");
    fixture.release_probe().await;
    yield_now().await;
    yield_now().await;
    advance(CHANNEL_RETIRE_GRACE).await;
    yield_now().await;
    let services = support::services(
        &support::borrowed(&variables),
        support::langfuse::real_builder(),
    );
    let (rebuilt_exporter, rebuilt) = support::langfuse::recording();
    let _reseeded = support::langfuse::seed(
        &services,
        &support::langfuse::environment_request(),
        rebuilt_exporter,
    );
    assert_eq!(rebuilt.installed(), 0);
}

#[tokio::test]
async fn the_success_span_carries_the_model_parameters() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    fixture
        .manager
        .async_log_success_event(support::event(support::success_payload(None)))
        .await
        .expect("the event logs");
    assert!(fixture.manager.logger.flush().await);
    let spans = fixture.spans.spans();
    let parameters = support::langfuse::object(support::langfuse::attribute_json(
        &spans,
        "langfuse.observation.model.parameters",
    ));
    assert!(parameters.contains_key("temperature"));
}
