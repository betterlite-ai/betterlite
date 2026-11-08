use std::sync::Arc;
use std::time::Duration;

use betterlite_logging::rubrik_delivery::config::{RubrikInit, RubrikSettings};
use betterlite_logging::rubrik_delivery::error::RubrikError;
use betterlite_logging::rubrik_delivery::logger::RubrikLogger;
use betterlite_logging::rubrik_delivery::post::post_prompt_moderation;
use betterlite_logging::rubrik_delivery::transport::{
    HttpTransport, LOGGING_TIMEOUT, MODERATION_TIMEOUT, Timeout,
};
use rstest::rstest;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio::task::JoinSet;
use tokio::time::sleep;

use super::support::{
    Environment, FixedClock, bare_init, empty_policy, init, logger, logger_with_environment,
    logging_timeout, moderation_timeout, number, payload,
};

#[rstest]
#[case::plain("https://rubrik.test", "https://rubrik.test")]
#[case::trailing_slash("https://rubrik.test/", "https://rubrik.test")]
#[case::trailing_slashes("https://rubrik.test///", "https://rubrik.test")]
#[case::v1_segment("https://rubrik.test/v1", "https://rubrik.test")]
#[case::v1_with_slash("https://rubrik.test/v1/", "https://rubrik.test")]
#[case::base_path("https://rubrik.test/base", "https://rubrik.test/base")]
#[case::port("http://127.0.0.1:8080", "http://127.0.0.1:8080")]
fn webhook_url_builds_all_three_endpoints(#[case] webhook_url: &str, #[case] base: &str) {
    let mut options = bare_init();
    options.api_base = Some(webhook_url.to_owned());
    let (logger, _transport) = logger(&options, Vec::new());
    assert_eq!(
        logger.settings.response_moderation_endpoint,
        format!("{base}/v1/after_completion/openai/v1")
    );
    assert_eq!(
        logger.settings.prompt_moderation_endpoint,
        format!("{base}/v1/before_prompt/openai/v1")
    );
    assert_eq!(
        logger.settings.logging_endpoint,
        format!("{base}/v1/litellm/batch")
    );
}

#[test]
fn one_v1_suffix_is_removed_after_trailing_slashes() {
    let mut options = bare_init();
    options.api_base = Some("https://rubrik.test/v1/v1/".to_owned());
    let (logger, _transport) = logger(&options, Vec::new());
    assert_eq!(
        logger.settings.logging_endpoint,
        "https://rubrik.test/v1/v1/litellm/batch"
    );
}

#[test]
fn moderation_timeout_defaults_to_five_and_two_seconds() {
    let (logger, _transport) = logger(&init("https://rubrik.test", "key"), Vec::new());
    assert_eq!(logger.settings.moderation_timeout, moderation_timeout());
    assert_eq!(logger.settings.logging_timeout, logging_timeout());
}

#[test]
fn configured_timeout_replaces_moderation_total_and_connect() {
    let mut options = init("https://rubrik.test", "key");
    options.timeout = Some(Duration::from_millis(2500));
    let (logger, _transport) = logger(&options, Vec::new());
    assert_eq!(
        logger.settings.moderation_timeout,
        Timeout {
            total: Duration::from_millis(2500),
            connect: Duration::from_millis(2500),
        }
    );
    assert_eq!(logger.settings.logging_timeout, LOGGING_TIMEOUT);
}

#[test]
fn environment_webhook_url_builds_the_endpoints_when_no_api_base_is_passed() {
    let values = Environment::from([(
        "RUBRIK_WEBHOOK_URL".to_owned(),
        "https://env.test/v1/".to_owned(),
    )]);
    let (logger, _transport) = logger_with_environment(&bare_init(), &values, Vec::new());
    assert_eq!(
        logger.settings.response_moderation_endpoint,
        "https://env.test/v1/after_completion/openai/v1"
    );
    assert_eq!(
        logger.settings.prompt_moderation_endpoint,
        "https://env.test/v1/before_prompt/openai/v1"
    );
    assert_eq!(
        logger.settings.logging_endpoint,
        "https://env.test/v1/litellm/batch"
    );
}

#[test]
fn timeout_constants_match_the_lite_llm_clients() {
    assert_eq!(
        MODERATION_TIMEOUT,
        Timeout {
            total: Duration::from_secs(5),
            connect: Duration::from_secs(2),
        }
    );
    assert_eq!(
        LOGGING_TIMEOUT,
        Timeout {
            total: Duration::from_secs(600),
            connect: Duration::from_secs(5),
        }
    );
    assert_eq!(number(MODERATION_TIMEOUT.total.as_secs_f64()), number(5.0));
}

#[test]
fn configured_request_timeout_replaces_the_logging_total() {
    let mut options = init("https://rubrik.test", "key");
    options.request_timeout = Some(Duration::from_secs(30));
    let (logger, _transport) = logger(&options, Vec::new());
    assert_eq!(
        logger.settings.logging_timeout,
        Timeout {
            total: Duration::from_secs(30),
            connect: Duration::from_secs(5),
        }
    );
    assert_eq!(logger.settings.moderation_timeout, moderation_timeout());
}

fn real_logger(options: &RubrikInit) -> Arc<RubrikLogger> {
    let settings = RubrikSettings::resolve(options, &Environment::new()).expect("settings resolve");
    let transport = HttpTransport::new(&empty_policy(), &settings).expect("clients build");
    Arc::new(
        RubrikLogger::new(
            options,
            &Environment::new(),
            Arc::new(transport),
            Arc::new(FixedClock(Duration::from_secs(1))),
        )
        .expect("the logger initializes"),
    )
}

struct StalledWebhook {
    base_url: String,
    tasks: JoinSet<()>,
}

async fn stalled_webhook(delay: Duration) -> StalledWebhook {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the listener binds");
    let address = listener.local_addr().expect("the listener has an address");
    let mut tasks = JoinSet::new();
    tasks.spawn(answer_after(listener, delay));
    StalledWebhook {
        base_url: format!("http://{address}"),
        tasks,
    }
}

async fn answer_after(listener: TcpListener, delay: Duration) {
    let Ok((mut stream, _peer)) = listener.accept().await else {
        return;
    };
    let mut request = [0_u8; 4096];
    if stream.read(&mut request).await.is_err() {
        return;
    }
    sleep(delay).await;
    let body = "{}";
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _write_result = stream.write_all(response.as_bytes()).await;
}

#[tokio::test]
async fn default_timeout_gives_up_on_a_stalled_moderation_webhook() {
    let mut webhook = stalled_webhook(Duration::from_secs(6)).await;
    let logger = real_logger(&init(&webhook.base_url, "key"));
    let error = post_prompt_moderation(&logger, &payload(r#"{"model":"gpt-4o"}"#))
        .await
        .expect_err("the default 5-second budget ends the stalled request");
    assert!(matches!(error, RubrikError::Transport { .. }));
    webhook.tasks.shutdown().await;
}

#[tokio::test]
async fn configured_timeout_lets_a_slow_moderation_webhook_answer() {
    let mut webhook = stalled_webhook(Duration::from_secs(6)).await;
    let mut options = init(&webhook.base_url, "key");
    options.timeout = Some(Duration::from_secs(10));
    let logger = real_logger(&options);
    let returned = post_prompt_moderation(&logger, &payload(r#"{"model":"gpt-4o"}"#))
        .await
        .expect("the configured 10-second budget lets the answer through");
    assert!(returned.is_empty());
    webhook.tasks.shutdown().await;
}
