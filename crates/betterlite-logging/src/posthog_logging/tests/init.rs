use std::time::Duration;

use rstest::rstest;

use crate::callback_logger::CallbackLogger as _;
use crate::posthog_logging::settings::{MAX_BATCH_SIZE, PostHogSettingsError, resolve};
use crate::posthog_logging::tests::support::{
    API_KEY, ScriptedTransport, default_settings, environment, event, fixed_id, logger_with,
    logger_with_shutdown, payload, queued, settle,
};

#[rstest]
#[case::default_url(&[("POSTHOG_API_KEY", API_KEY)], "https://us.i.posthog.com")]
#[case::custom_url(
    &[("POSTHOG_API_KEY", API_KEY), ("POSTHOG_API_URL", "https://eu.i.posthog.com")],
    "https://eu.i.posthog.com"
)]
#[case::trailing_slashes(
    &[("POSTHOG_API_KEY", API_KEY), ("POSTHOG_API_URL", "https://eu.i.posthog.com///")],
    "https://eu.i.posthog.com"
)]
#[case::empty_url_falls_back(
    &[("POSTHOG_API_KEY", API_KEY), ("POSTHOG_API_URL", "")],
    "https://us.i.posthog.com"
)]
fn posthog_init_resolves_the_host(#[case] pairs: &[(&str, &str)], #[case] host: &str) {
    let settings = resolve(&environment(pairs)).expect("the settings resolve");
    assert_eq!(settings.api_url, host);
    assert_eq!(settings.capture_url, format!("{host}/batch/"));
}

#[test]
fn posthog_init_fixes_the_batch_size_at_one_hundred() {
    let settings = resolve(&environment(&[
        ("POSTHOG_API_KEY", API_KEY),
        ("DEFAULT_BATCH_SIZE", "7"),
        ("DEFAULT_FLUSH_INTERVAL_SECONDS", "12"),
    ]))
    .expect("the settings resolve");
    assert_eq!(settings.batch.batch_size, MAX_BATCH_SIZE);
    assert_eq!(MAX_BATCH_SIZE, 100);
    assert_eq!(settings.batch.flush_interval, Duration::from_secs(12));
    assert_eq!(settings.batch.max_queue_size, 50_000);
}

#[test]
fn posthog_init_defaults_the_flush_interval_to_five_seconds() {
    let settings = default_settings();
    assert_eq!(settings.batch.flush_interval, Duration::from_secs(5));
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn posthog_init_requires_an_api_key(#[case] key: Option<&str>) {
    let pairs: Vec<(&str, &str)> =
        key.map_or_else(Vec::new, |value| vec![("POSTHOG_API_KEY", value)]);
    let error = resolve(&environment(&pairs)).expect_err("the key is missing");
    assert!(matches!(error, PostHogSettingsError::MissingApiKey));
}

#[rstest]
#[case::true_enables(&[("POSTHOG_MOCK", "true")], true)]
#[case::false_disables(&[("POSTHOG_MOCK", "false")], false)]
#[case::empty_disables(&[("POSTHOG_MOCK", "")], false)]
fn posthog_init_reads_the_mock_flag(#[case] pairs: &[(&str, &str)], #[case] expected: bool) {
    let mut all = vec![("POSTHOG_API_KEY", API_KEY)];
    all.extend_from_slice(pairs);
    let settings = resolve(&environment(&all)).expect("the settings resolve");
    assert_eq!(settings.mock_mode, expected);
}

#[test]
fn posthog_init_rejects_a_non_boolean_mock_flag() {
    let error = resolve(&environment(&[
        ("POSTHOG_API_KEY", API_KEY),
        ("POSTHOG_MOCK", "sometimes"),
    ]))
    .expect_err("the flag is invalid");
    assert!(error.to_string().contains("POSTHOG_MOCK"));
}

#[tokio::test]
async fn posthog_init_registers_the_exit_flush() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, tasks, shutdown) = logger_with_shutdown(default_settings(), &transport, fixed_id);
    assert_eq!(tasks.len(), 1);
    let _enqueued = logger.sink.queue.enqueue(queued(&payload()));
    shutdown.cancel();
    settle(&tasks).await;
    assert_eq!(transport.urls(), vec!["https://us.i.posthog.com/batch/"]);
}

#[tokio::test]
async fn posthog_init_without_a_shutdown_token_registers_no_task() {
    let transport = ScriptedTransport::new(&[]);
    let (logger, tasks) = logger_with(default_settings(), &transport, fixed_id);
    assert_eq!(tasks.len(), 0);
    logger
        .log_success_event(&event(payload()))
        .expect("the sync event is accepted");
    settle(&tasks).await;
    assert_eq!(transport.urls().len(), 1);
}
