use std::sync::Arc;
use std::time::Duration;

use betterlite_logging::rubrik_delivery::config::{
    DEFAULT_BATCH_SIZE, DEFAULT_GUARDRAIL_NAME, MAX_QUEUE_SIZE, RubrikEventHook,
    SUPPORTED_EVENT_HOOKS, StreamFlags,
};
use betterlite_logging::rubrik_delivery::error::RubrikError;
use betterlite_logging::rubrik_delivery::transport::{
    HttpTransport, LOGGING_TIMEOUT, MODERATION_TIMEOUT, Timeout,
};
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use rstest::rstest;

use super::support::{
    Environment, FixedClock, bare_init, build, capture, empty_policy, environment, init, logger,
    logger_with_environment, logging_timeout, moderation_timeout, number,
};

#[test]
fn defaults_fill_every_field() {
    let (logger, _transport) = logger(&init("https://rubrik.test", "test-key"), Vec::new());
    let settings = &logger.settings;
    assert_eq!(settings.guardrail_name, DEFAULT_GUARDRAIL_NAME);
    assert_eq!(settings.event_hook, RubrikEventHook::PostCall);
    assert_eq!(
        SUPPORTED_EVENT_HOOKS,
        [RubrikEventHook::PreCall, RubrikEventHook::PostCall]
    );
    assert!(!settings.default_on);
    assert_eq!(number(settings.sampling_rate), number(1.0));
    assert_eq!(settings.batch_size, DEFAULT_BATCH_SIZE);
    assert_eq!(settings.max_queue_size, MAX_QUEUE_SIZE);
}

#[test]
fn stream_flags_are_true_and_the_queue_starts_empty() {
    let (logger, transport) = logger(&init("https://rubrik.test", "test-key"), Vec::new());
    assert_eq!(
        logger.settings.stream,
        StreamFlags {
            end_of_stream_only: true,
            buffer_until_moderated: true,
        }
    );
    assert_eq!(logger.settings.moderation_timeout, moderation_timeout());
    assert_eq!(logger.settings.logging_timeout, logging_timeout());
    let state = logger.state();
    assert_eq!(state.length, 0);
    assert_eq!(state.dropped_since_warning, 0);
    assert_eq!(state.last_drop_warning_time, Duration::ZERO);
    assert_eq!(state.last_flush_time, Duration::from_secs(1));
    assert_eq!(transport.call_count(), 0);
}

#[test]
fn environment_supplies_the_integration_settings() {
    let values = environment(&[
        ("RUBRIK_API_KEY", "env-key"),
        ("RUBRIK_WEBHOOK_URL", "https://env.test"),
        ("RUBRIK_SAMPLING_RATE", "0.25"),
        ("RUBRIK_BATCH_SIZE", "7"),
    ]);
    let (logger, _transport) = logger_with_environment(&bare_init(), &values, Vec::new());
    let settings = &logger.settings;
    assert!(settings.api_key.is_some());
    assert_eq!(number(settings.sampling_rate), number(0.25));
    assert_eq!(settings.batch_size, 7);
    assert_eq!(
        settings.response_moderation_endpoint,
        "https://env.test/v1/after_completion/openai/v1"
    );
    assert_eq!(
        settings.prompt_moderation_endpoint,
        "https://env.test/v1/before_prompt/openai/v1"
    );
    assert_eq!(
        settings.logging_endpoint,
        "https://env.test/v1/litellm/batch"
    );
    let headers = settings.headers().expect("headers build");
    assert_eq!(
        headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some("Bearer env-key")
    );
}

#[rstest]
#[case::fraction("0.25", 0.25_f64)]
#[case::above_one("2", 1.0_f64)]
#[case::below_zero("-1", 0.0_f64)]
#[case::invalid_text("abc", 1.0_f64)]
#[case::empty_text("", 1.0_f64)]
#[case::padded_text(" 0.5 ", 0.5_f64)]
fn sampling_rate_parses_clamps_and_falls_back(#[case] text: &str, #[case] expected: f64) {
    let values = environment(&[("RUBRIK_SAMPLING_RATE", text)]);
    let (logger, _transport) =
        logger_with_environment(&init("https://rubrik.test", "key"), &values, Vec::new());
    assert_eq!(number(logger.settings.sampling_rate), number(expected));
}

#[test]
fn invalid_sampling_rate_warns_and_uses_one() {
    let values = environment(&[("RUBRIK_SAMPLING_RATE", "abc")]);
    let (result, warnings) = capture(|| {
        build(
            &init("https://rubrik.test", "key"),
            &values,
            Arc::new(FixedClock(Duration::from_secs(1))),
            Vec::new(),
        )
    });
    let (logger, _transport) = result.expect("initialization succeeds");
    assert_eq!(number(logger.settings.sampling_rate), number(1.0));
    assert_eq!(warnings.count_containing("Invalid RUBRIK_SAMPLING_RATE"), 1);
}

#[test]
fn empty_sampling_rate_warns_and_uses_one() {
    let values = environment(&[("RUBRIK_SAMPLING_RATE", "")]);
    let (result, warnings) = capture(|| {
        build(
            &init("https://rubrik.test", "key"),
            &values,
            Arc::new(FixedClock(Duration::from_secs(1))),
            Vec::new(),
        )
    });
    let (logger, _transport) = result.expect("initialization succeeds");
    assert_eq!(number(logger.settings.sampling_rate), number(1.0));
    assert_eq!(warnings.count_containing("Invalid RUBRIK_SAMPLING_RATE"), 1);
}

#[test]
fn out_of_range_sampling_rate_clamps_with_a_warning() {
    let values = environment(&[("RUBRIK_SAMPLING_RATE", "2")]);
    let (result, warnings) = capture(|| {
        build(
            &init("https://rubrik.test", "key"),
            &values,
            Arc::new(FixedClock(Duration::from_secs(1))),
            Vec::new(),
        )
    });
    let (logger, _transport) = result.expect("initialization succeeds");
    assert_eq!(number(logger.settings.sampling_rate), number(1.0));
    assert_eq!(warnings.count_containing("clamped"), 1);
}

#[rstest]
#[case::positive("7", 7)]
#[case::zero("0", DEFAULT_BATCH_SIZE)]
#[case::negative("-3", DEFAULT_BATCH_SIZE)]
#[case::invalid_text("abc", DEFAULT_BATCH_SIZE)]
#[case::empty_text("", DEFAULT_BATCH_SIZE)]
fn batch_size_accepts_positive_integers_only(#[case] text: &str, #[case] expected: usize) {
    let values = environment(&[("RUBRIK_BATCH_SIZE", text)]);
    let (logger, _transport) =
        logger_with_environment(&init("https://rubrik.test", "key"), &values, Vec::new());
    assert_eq!(logger.settings.batch_size, expected);
}

#[test]
fn non_positive_batch_size_warns_and_keeps_the_default() {
    let values = environment(&[("RUBRIK_BATCH_SIZE", "0")]);
    let (result, warnings) = capture(|| {
        build(
            &init("https://rubrik.test", "key"),
            &values,
            Arc::new(FixedClock(Duration::from_secs(1))),
            Vec::new(),
        )
    });
    let (logger, _transport) = result.expect("initialization succeeds");
    assert_eq!(logger.settings.batch_size, DEFAULT_BATCH_SIZE);
    assert_eq!(warnings.count_containing("must be > 0"), 1);
}

#[test]
fn non_integer_batch_size_warns_and_keeps_the_default() {
    let values = environment(&[("RUBRIK_BATCH_SIZE", "abc")]);
    let (result, warnings) = capture(|| {
        build(
            &init("https://rubrik.test", "key"),
            &values,
            Arc::new(FixedClock(Duration::from_secs(1))),
            Vec::new(),
        )
    });
    let (logger, _transport) = result.expect("initialization succeeds");
    assert_eq!(logger.settings.batch_size, DEFAULT_BATCH_SIZE);
    assert_eq!(warnings.count_containing("Invalid RUBRIK_BATCH_SIZE"), 1);
}

#[test]
fn missing_webhook_url_fails_with_the_lite_llm_message() {
    let result = build(
        &bare_init(),
        &Environment::new(),
        Arc::new(FixedClock(Duration::from_secs(1))),
        Vec::new(),
    );
    let error = result.err().expect("initialization fails");
    assert!(matches!(error, RubrikError::MissingWebhookUrl));
    assert_eq!(
        error.to_string(),
        "Rubrik webhook URL not configured. Set RUBRIK_WEBHOOK_URL or pass api_base."
    );
}

#[test]
fn missing_api_key_warns_and_sends_unauthenticated() {
    let mut options = bare_init();
    options.api_base = Some("https://rubrik.test".to_owned());
    let (result, warnings) = capture(|| {
        build(
            &options,
            &Environment::new(),
            Arc::new(FixedClock(Duration::from_secs(1))),
            Vec::new(),
        )
    });
    let (logger, _transport) = result.expect("initialization succeeds");
    assert!(logger.settings.api_key.is_none());
    let headers = logger.settings.headers().expect("headers build");
    assert!(headers.get(AUTHORIZATION).is_none());
    assert_eq!(
        headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/json")
    );
    assert_eq!(warnings.count_containing("No API key configured"), 1);
}

#[test]
fn constructor_api_key_wins_over_environment() {
    let values = environment(&[("RUBRIK_API_KEY", "env-key")]);
    let (logger, _transport) = logger_with_environment(
        &init("https://rubrik.test", "init-key"),
        &values,
        Vec::new(),
    );
    let headers = logger.settings.headers().expect("headers build");
    assert_eq!(
        headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
        Some("Bearer init-key")
    );
}

#[test]
fn constructor_api_base_wins_over_environment() {
    let values = environment(&[("RUBRIK_WEBHOOK_URL", "https://env.test")]);
    let (logger, _transport) =
        logger_with_environment(&init("https://init.test", "key"), &values, Vec::new());
    assert_eq!(
        logger.settings.logging_endpoint,
        "https://init.test/v1/litellm/batch"
    );
}

#[test]
fn http_clients_build_with_the_lite_llm_timeouts() {
    let transport = HttpTransport::new(&empty_policy(), MODERATION_TIMEOUT, LOGGING_TIMEOUT)
        .expect("clients build");
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
    drop(transport);
}
