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
    assert_eq!(
        format!("{logger:?}"),
        r#"LangFuseLogger { project_id: Some("mock-project-id"), stopped: false, .. }"#
    );
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
        .expect_err("the prompt cache ttl is refused");
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
        .expect_err("the environment is refused");
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
        Arc::new(support::FixedIds),
    );
    let request = support::request("pk-failing", "sk-failing");
    let (exporter, recording) = support::recording();
    let probe = support::seed(&services, &request, exporter);
    release_langfuse_tracing(&probe, CHANNEL_RETIRE_GRACE).await;
    let error = LangFuseLogger::new(&services, &request)
        .await
        .expect_err("the client build fails");
    assert!(matches!(error, LangfuseLoggerError::Client(_)));
    assert_eq!(recording.shut_down(), 1);
    let _reservation = services.budget.reserve().expect("the slot is free");
}

#[tokio::test]
async fn an_unusable_timeout_is_named() {
    let services = support::services(&[("LANGFUSE_TIMEOUT", "-1"), ("LANGFUSE_MOCK", "true")], 10);
    let error = LangFuseLogger::new(&services, &support::environment_request())
        .await
        .expect_err("the timeout is refused");
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
    let _client = builder.build(config).expect("the client builds");
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
