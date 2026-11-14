use std::sync::Arc;

use super::support;
use crate::callback_logger::CallbackLogger;
use crate::langfuse_prompt_management::LangfusePromptManagement;
use crate::langfuse_prompt_management::clients::PromptClientRequest;

#[tokio::test]
async fn the_manager_owns_a_tracing_channel_that_receives_its_events() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    let event = support::event(support::success_payload(Some(support::CALL_ID)));
    fixture
        .manager
        .async_log_success_event(event)
        .await
        .expect("the event logs");
    assert!(fixture.manager.logger.flush().await);
    assert_eq!(fixture.spans.spans().len(), 1);
}

#[tokio::test]
async fn the_manager_owns_a_prompt_client_factory() {
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
    let request = PromptClientRequest {
        public_key: None,
        secret: None,
        secret_key: None,
        host: None,
        flush_interval: 1,
        allow_env_credentials: true,
    };
    manager
        .clients
        .client_init(&manager.services, &request)
        .expect("the prompt client builds");
    let configs = builder.configs();
    assert_eq!(configs.len(), 2);
    assert_eq!(configs[0].public_key.as_deref(), Some(support::PUBLIC_KEY));
    assert_eq!(configs[1].public_key.as_deref(), Some(support::PUBLIC_KEY));
}
