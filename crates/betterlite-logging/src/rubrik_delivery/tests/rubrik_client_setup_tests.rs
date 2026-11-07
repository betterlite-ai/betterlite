use std::time::Duration;

use betterlite_logging::rubrik_delivery::transport::{
    LOGGING_TIMEOUT, MODERATION_TIMEOUT, Timeout,
};
use rstest::rstest;

use super::support::{
    Environment, bare_init, init, logger, logger_with_environment, logging_timeout,
    moderation_timeout, number,
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
