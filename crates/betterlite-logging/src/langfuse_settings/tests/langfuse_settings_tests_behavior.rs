use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::langfuse_settings::Settings;
use rstest::rstest;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::langfuse_settings::export::{
    check_prompt_cache_ttl, flush_at, flush_interval, max_retries, prompt_cache_ttl_seconds,
    sample_rate, timeout_seconds,
};
use crate::langfuse_settings::{
    CLOUD_HOST, CredentialRequest, Credentials, LangfuseSettingsError, debug, mock_mode, release,
    resolve_credentials, resolve_environment, resolve_host, warn_if_upstream_configured,
};

fn load(variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load("")
        .expect("the langfuse settings load")
        .sections
        .langfuse_settings
}

fn texts(messages: &[&str]) -> Vec<String> {
    messages
        .iter()
        .map(|message| (*message).to_owned())
        .collect()
}

#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<(Level, String)>>>);

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let level = *event.metadata().level();
        if level != Level::WARN && level != Level::INFO {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((level, message.0));
    }
}

impl Logs {
    fn messages(&self, level: Level) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(recorded, _)| *recorded == level)
            .map(|(_, message)| message.clone())
            .collect()
    }
}

fn capture<T, F>(run: F) -> (T, Logs)
where
    F: FnOnce() -> T,
{
    let logs = Logs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let value = with_default(subscriber, run);
    (value, logs)
}

fn warnings(logs: &Logs) -> Vec<String> {
    logs.messages(Level::WARN)
}

fn infos(logs: &Logs) -> Vec<String> {
    logs.messages(Level::INFO)
}

fn resolved_secret(credentials: &Credentials) -> Option<&str> {
    credentials
        .secret_key
        .as_ref()
        .map(secrecy::ExposeSecret::expose_secret)
}

#[rstest]
#[case::the_arguments_win(
    &[("LANGFUSE_PUBLIC_KEY", "env-public"), ("LANGFUSE_SECRET_KEY", "env-secret")],
    CredentialRequest { public_key: Some("arg-public"), secret: Some("arg-secret"), secret_key: None, host: None, allow_env_credentials: true },
    (Some("arg-public"), Some("arg-secret"), CLOUD_HOST)
)]
#[case::missing_arguments_come_from_the_environment(
    &[("LANGFUSE_PUBLIC_KEY", "env-public"), ("LANGFUSE_SECRET_KEY", "env-secret")],
    CredentialRequest { public_key: None, secret: None, secret_key: None, host: None, allow_env_credentials: true },
    (Some("env-public"), Some("env-secret"), CLOUD_HOST)
)]
#[case::secret_key_comes_before_the_environment(
    &[("LANGFUSE_SECRET_KEY", "env-secret")],
    CredentialRequest { public_key: None, secret: None, secret_key: Some("arg-key"), host: None, allow_env_credentials: true },
    (None, Some("arg-key"), CLOUD_HOST)
)]
#[case::the_secret_argument_comes_before_secret_key(
    &[],
    CredentialRequest { public_key: None, secret: Some("arg-secret"), secret_key: Some("arg-key"), host: None, allow_env_credentials: true },
    (None, Some("arg-secret"), CLOUD_HOST)
)]
#[case::an_empty_argument_falls_through(
    &[("LANGFUSE_PUBLIC_KEY", "env-public"), ("LANGFUSE_SECRET_KEY", "env-secret")],
    CredentialRequest { public_key: Some(""), secret: Some(""), secret_key: Some(""), host: None, allow_env_credentials: true },
    (Some("env-public"), Some("env-secret"), CLOUD_HOST)
)]
#[case::an_empty_environment_credential_is_kept(
    &[("LANGFUSE_PUBLIC_KEY", ""), ("LANGFUSE_SECRET_KEY", "")],
    CredentialRequest { public_key: None, secret: None, secret_key: None, host: None, allow_env_credentials: true },
    (Some(""), Some(""), CLOUD_HOST)
)]
#[case::arguments_still_win_when_a_host_blocks_the_environment(
    &[("LANGFUSE_PUBLIC_KEY", "env-public"), ("LANGFUSE_SECRET_KEY", "env-secret"), ("LANGFUSE_HOST", "env-host")],
    CredentialRequest { public_key: Some("arg-public"), secret: Some("arg-secret"), secret_key: None, host: Some("https://given"), allow_env_credentials: false },
    (Some("arg-public"), Some("arg-secret"), "https://given")
)]
#[case::the_environment_is_not_read(
    &[("LANGFUSE_PUBLIC_KEY", "env-public"), ("LANGFUSE_SECRET_KEY", "env-secret"), ("LANGFUSE_HOST", "env-host")],
    CredentialRequest { public_key: None, secret: None, secret_key: None, host: Some("https://given"), allow_env_credentials: false },
    (None, None, "https://given")
)]
#[case::an_empty_host_still_blocks_the_environment(
    &[("LANGFUSE_PUBLIC_KEY", "env-public"), ("LANGFUSE_SECRET_KEY", "env-secret"), ("LANGFUSE_HOST", "env-host")],
    CredentialRequest { public_key: None, secret: None, secret_key: None, host: Some(""), allow_env_credentials: false },
    (None, None, "http://env-host")
)]
fn credentials_resolve_from_the_arguments_then_the_environment(
    #[case] variables: &[(&str, &str)],
    #[case] request: CredentialRequest<'_>,
    #[case] expected: (Option<&str>, Option<&str>, &str),
) {
    let settings = load(variables);
    let credentials = resolve_credentials(&settings, request);

    assert_eq!(credentials.public_key.as_deref(), expected.0);
    assert_eq!(resolved_secret(&credentials), expected.1);
    assert_eq!(credentials.host, expected.2);
}

#[test]
fn credentials_read_the_environment_when_no_host_blocks_it() {
    let settings = load(&[
        ("LANGFUSE_PUBLIC_KEY", "env-public"),
        ("LANGFUSE_SECRET_KEY", "env-secret"),
    ]);
    let credentials = resolve_credentials(
        &settings,
        CredentialRequest {
            public_key: None,
            secret: None,
            secret_key: None,
            host: None,
            allow_env_credentials: false,
        },
    );

    assert_eq!(credentials.public_key.as_deref(), Some("env-public"));
    assert_eq!(resolved_secret(&credentials), Some("env-secret"));
}

#[rstest]
#[case::the_argument_wins(
    &[("LANGFUSE_HOST", "env-host"), ("LANGFUSE_BASE_URL", "https://base-host")],
    Some("https://arg-host"), "https://arg-host"
)]
#[case::an_empty_argument_falls_to_langfuse_host(
    &[("LANGFUSE_HOST", "env-host"), ("LANGFUSE_BASE_URL", "https://base-host")],
    Some(""), "http://env-host"
)]
#[case::langfuse_host_wins_over_the_base_url(
    &[("LANGFUSE_HOST", "https://env-host"), ("LANGFUSE_BASE_URL", "https://base-host")],
    None, "https://env-host"
)]
#[case::an_empty_langfuse_host_falls_to_the_base_url(
    &[("LANGFUSE_HOST", ""), ("LANGFUSE_BASE_URL", "https://base-host")],
    None, "https://base-host"
)]
#[case::the_cloud_host_is_the_last_fallback(&[], None, CLOUD_HOST)]
#[case::an_http_scheme_is_kept(&[], Some("http://langfuse.local"), "http://langfuse.local")]
#[case::a_missing_scheme_gets_http(&[], Some("langfuse.internal:3000"), "http://langfuse.internal:3000")]
#[case::a_scheme_is_case_sensitive(&[], Some("HTTPS://langfuse.local"), "http://HTTPS://langfuse.local")]
fn hosts_follow_the_fallbacks(
    #[case] variables: &[(&str, &str)],
    #[case] host: Option<&str>,
    #[case] expected: &str,
) {
    let settings = load(variables);

    assert_eq!(resolve_host(&settings, host), expected);
}

#[rstest]
#[case::an_explicit_value_is_trimmed(&[], Some("  production  "), Some("production"))]
#[case::an_empty_explicit_value_falls_to_the_variable(
    &[("LANGFUSE_TRACING_ENVIRONMENT", "qa")],
    Some(""),
    Some("qa")
)]
#[case::a_spaces_only_explicit_value_falls_to_the_variable(
    &[("LANGFUSE_TRACING_ENVIRONMENT", "qa")],
    Some("   "),
    Some("qa")
)]
#[case::the_variable_is_trimmed(&[("LANGFUSE_TRACING_ENVIRONMENT", "  qa  ")], None, Some("qa"))]
#[case::an_empty_variable_gives_none(&[("LANGFUSE_TRACING_ENVIRONMENT", "")], None, None)]
#[case::an_unset_variable_gives_none(&[], None, None)]
fn the_deployment_environment_falls_to_the_variable(
    #[case] variables: &[(&str, &str)],
    #[case] explicit: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let settings = load(variables);
    let (resolved, logs) = capture(|| resolve_environment(&settings, explicit));

    assert_eq!(
        resolved.expect("a valid environment resolves").as_deref(),
        expected
    );
    assert_eq!(warnings(&logs), Vec::<String>::new());
}

#[test]
fn an_invalid_explicit_environment_fails() {
    let settings = load(&[]);

    let error = resolve_environment(&settings, Some("Bad Env"))
        .expect_err("an uppercase explicit environment is rejected");

    assert!(matches!(error, LangfuseSettingsError::Environment { .. }));
}

#[test]
fn an_invalid_variable_warns_and_sends_traces_to_the_default() {
    let settings = load(&[("LANGFUSE_TRACING_ENVIRONMENT", "Bad Env")]);
    let (resolved, logs) = capture(|| resolve_environment(&settings, None));

    assert_eq!(
        resolved.expect("the fallback resolves").as_deref(),
        Some("default")
    );
    assert_eq!(
        warnings(&logs),
        texts(&[
            "Ignoring invalid LANGFUSE_TRACING_ENVIRONMENT='Bad Env' for the langfuse callback: invalid callback field langfuse_environment. Traces will be sent to Langfuse's default environment."
        ])
    );
}

#[test]
fn a_spaces_only_variable_warns_and_sends_traces_to_the_default() {
    let settings = load(&[("LANGFUSE_TRACING_ENVIRONMENT", "   ")]);
    let (resolved, logs) = capture(|| resolve_environment(&settings, None));

    assert_eq!(
        resolved.expect("the fallback resolves").as_deref(),
        Some("default")
    );
    assert_eq!(
        warnings(&logs),
        texts(&[
            "Ignoring invalid LANGFUSE_TRACING_ENVIRONMENT='   ' for the langfuse callback: invalid callback field langfuse_environment. Traces will be sent to Langfuse's default environment."
        ])
    );
}

#[rstest]
#[case::true_is_debug(&[("LANGFUSE_DEBUG", "true")], true)]
#[case::one_is_debug(&[("LANGFUSE_DEBUG", "1")], true)]
#[case::case_and_spaces_are_ignored(&[("LANGFUSE_DEBUG", " TRUE ")], true)]
#[case::yes_is_debug(&[("LANGFUSE_DEBUG", "yes")], true)]
#[case::on_is_debug(&[("LANGFUSE_DEBUG", "on")], true)]
#[case::zero_is_not_debug(&[("LANGFUSE_DEBUG", "0")], false)]
#[case::false_is_not_debug(&[("LANGFUSE_DEBUG", "false")], false)]
#[case::no_is_not_debug(&[("LANGFUSE_DEBUG", "no")], false)]
#[case::an_empty_value_is_not_debug(&[("LANGFUSE_DEBUG", "")], false)]
#[case::an_unset_value_is_not_debug(&[], false)]
fn debug_reads_the_shared_parser(#[case] variables: &[(&str, &str)], #[case] expected: bool) {
    let settings = load(variables);

    assert_eq!(
        debug(&settings).expect("a boolean spelling parses"),
        expected
    );
}

#[test]
fn an_unrecognized_debug_value_fails_naming_the_variable() {
    let settings = load(&[("LANGFUSE_DEBUG", "maybe")]);

    let error = debug(&settings).expect_err("an unknown spelling fails");

    assert!(matches!(&error, LangfuseSettingsError::Flag { .. }));
    assert!(error.to_string().contains("LANGFUSE_DEBUG"));
}

#[rstest]
#[case::true_enables_mock_mode(&[("LANGFUSE_MOCK", "true")], true)]
#[case::case_and_spaces_are_ignored(&[("LANGFUSE_MOCK", " TRUE ")], true)]
#[case::one_enables_mock_mode(&[("LANGFUSE_MOCK", "1")], true)]
#[case::yes_enables_mock_mode(&[("LANGFUSE_MOCK", "yes")], true)]
#[case::false_is_off(&[("LANGFUSE_MOCK", "false")], false)]
#[case::off_is_off(&[("LANGFUSE_MOCK", "off")], false)]
#[case::an_empty_value_is_off(&[("LANGFUSE_MOCK", "")], false)]
#[case::an_unset_value_is_off(&[], false)]
fn mock_mode_reads_the_shared_parser(#[case] variables: &[(&str, &str)], #[case] expected: bool) {
    let settings = load(variables);
    let (enabled, logs) = capture(|| mock_mode(&settings));
    let parsed = enabled.expect("a boolean spelling parses");

    assert_eq!(parsed, expected);
    let expected_infos = if expected {
        texts(&["LANGFUSE Mock Mode: ENABLED - API calls will be mocked"])
    } else {
        Vec::new()
    };
    assert_eq!(infos(&logs), expected_infos);
}

#[test]
fn an_unrecognized_mock_value_fails_naming_the_variable() {
    let settings = load(&[("LANGFUSE_MOCK", "maybe")]);

    let error = mock_mode(&settings).expect_err("an unknown spelling fails");

    assert!(matches!(&error, LangfuseSettingsError::Flag { .. }));
    assert!(error.to_string().contains("LANGFUSE_MOCK"));
}

#[test]
fn the_upstream_warning_fires_when_the_variable_is_set() {
    let settings = load(&[("UPSTREAM_LANGFUSE_SECRET_KEY", "secret")]);
    let ((), logs) = capture(|| warn_if_upstream_configured(&settings));

    assert_eq!(
        warnings(&logs),
        texts(&[
            "UPSTREAM_LANGFUSE_* is no longer supported: the langfuse callback moved to SDK v4, which has no second ingestion client. The values are ignored."
        ])
    );
}

#[test]
fn the_upstream_warning_fires_for_an_empty_value_too() {
    let settings = load(&[("UPSTREAM_LANGFUSE_SECRET_KEY", "")]);
    let ((), logs) = capture(|| warn_if_upstream_configured(&settings));

    assert_eq!(warnings(&logs).len(), 1);
}

#[test]
fn the_upstream_warning_is_silent_when_unset() {
    let settings = load(&[]);
    let ((), logs) = capture(|| warn_if_upstream_configured(&settings));

    assert_eq!(warnings(&logs), Vec::<String>::new());
}

#[rstest]
#[case::release_wins(&[("LANGFUSE_RELEASE", "release-9"), ("GITHUB_SHA", "sha")], Some("release-9"))]
#[case::an_empty_release_falls_through(&[("LANGFUSE_RELEASE", ""), ("GITHUB_SHA", "sha")], Some("sha"))]
#[case::a_spaces_release_is_kept(&[("LANGFUSE_RELEASE", "   "), ("GITHUB_SHA", "sha")], Some("   "))]
#[case::render_git_commit(&[("RENDER_GIT_COMMIT", "render")], Some("render"))]
#[case::ci_commit_sha(&[("CI_COMMIT_SHA", "ci")], Some("ci"))]
#[case::circle_sha1(&[("CIRCLE_SHA1", "circle")], Some("circle"))]
#[case::source_version(&[("SOURCE_VERSION", "source")], Some("source"))]
#[case::travis_commit(&[("TRAVIS_COMMIT", "travis")], Some("travis"))]
#[case::git_commit(&[("GIT_COMMIT", "git")], Some("git"))]
#[case::github_sha(&[("GITHUB_SHA", "github")], Some("github"))]
#[case::bitbucket_commit(&[("BITBUCKET_COMMIT", "bitbucket")], Some("bitbucket"))]
#[case::build_sourceversion(&[("BUILD_SOURCEVERSION", "build")], Some("build"))]
#[case::drone_commit_sha(&[("DRONE_COMMIT_SHA", "drone")], Some("drone"))]
#[case::render_git_commit_comes_first(
    &[("DRONE_COMMIT_SHA", "drone"), ("GITHUB_SHA", "github"), ("RENDER_GIT_COMMIT", "render")],
    Some("render")
)]
#[case::an_empty_commit_counts_as_set(&[("RENDER_GIT_COMMIT", ""), ("GITHUB_SHA", "sha")], Some(""))]
#[case::nothing_is_set(&[], None)]
fn release_walks_the_variables_in_order(
    #[case] variables: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    let settings = load(variables);

    assert_eq!(release(&settings).as_deref(), expected);
}

#[derive(Clone, Copy)]
enum Reading {
    FlushInterval,
    FlushAt,
    SampleRate,
    MaxRetries,
    PromptCacheTtl,
}

#[derive(Debug, PartialEq)]
enum Expected {
    Whole(u64),
    Batch(usize),
    Retries(u32),
    Rate(u64),
}

impl Expected {
    fn rate(value: f64) -> Self {
        Self::Rate(value.to_bits())
    }
}

fn read(reading: Reading, settings: &Settings) -> Expected {
    match reading {
        Reading::FlushInterval => Expected::Whole(flush_interval(settings, 7)),
        Reading::FlushAt => Expected::Batch(flush_at(settings)),
        Reading::SampleRate => Expected::rate(sample_rate(settings)),
        Reading::MaxRetries => Expected::Retries(max_retries(settings)),
        Reading::PromptCacheTtl => Expected::rate(prompt_cache_ttl_seconds(settings)),
    }
}

#[rstest]
#[case::flush_interval_whole_seconds(Reading::FlushInterval, &[("LANGFUSE_FLUSH_INTERVAL", "5")], Expected::Whole(5_u64), &[])]
#[case::flush_interval_spaces_are_trimmed(Reading::FlushInterval, &[("LANGFUSE_FLUSH_INTERVAL", " 5 ")], Expected::Whole(5_u64), &[])]
#[case::flush_interval_zero_falls_back(Reading::FlushInterval, &[("LANGFUSE_FLUSH_INTERVAL", "0")], Expected::Whole(7_u64), &["LANGFUSE_FLUSH_INTERVAL='0' is not a whole number of seconds above 0; flushing every 7 s"])]
#[case::flush_interval_negative_falls_back(Reading::FlushInterval, &[("LANGFUSE_FLUSH_INTERVAL", "-5")], Expected::Whole(7_u64), &["LANGFUSE_FLUSH_INTERVAL='-5' is not a whole number of seconds above 0; flushing every 7 s"])]
#[case::flush_interval_fraction_falls_back(Reading::FlushInterval, &[("LANGFUSE_FLUSH_INTERVAL", "1.5")], Expected::Whole(7_u64), &["LANGFUSE_FLUSH_INTERVAL='1.5' is not a whole number of seconds above 0; flushing every 7 s"])]
#[case::flush_interval_text_falls_back(Reading::FlushInterval, &[("LANGFUSE_FLUSH_INTERVAL", "soon")], Expected::Whole(7_u64), &["LANGFUSE_FLUSH_INTERVAL='soon' is not a whole number of seconds above 0; flushing every 7 s"])]
#[case::flush_interval_spaces_fall_back(Reading::FlushInterval, &[("LANGFUSE_FLUSH_INTERVAL", "   ")], Expected::Whole(7_u64), &["LANGFUSE_FLUSH_INTERVAL='   ' is not a whole number of seconds above 0; flushing every 7 s"])]
#[case::flush_interval_empty_is_silent(Reading::FlushInterval, &[("LANGFUSE_FLUSH_INTERVAL", "")], Expected::Whole(7_u64), &[])]
#[case::flush_interval_unset_is_silent(Reading::FlushInterval, &[], Expected::Whole(7_u64), &[])]
#[case::flush_interval_overflow_saturates(Reading::FlushInterval, &[("LANGFUSE_FLUSH_INTERVAL", "18446744073709551616")], Expected::Whole(u64::MAX), &[])]
#[case::flush_at_default(Reading::FlushAt, &[], Expected::Batch(512_usize), &[])]
#[case::flush_at_one(Reading::FlushAt, &[("LANGFUSE_FLUSH_AT", "1")], Expected::Batch(1_usize), &[])]
#[case::flush_at_maximum(Reading::FlushAt, &[("LANGFUSE_FLUSH_AT", "100000")], Expected::Batch(100_000_usize), &[])]
#[case::flush_at_spaces_are_trimmed(Reading::FlushAt, &[("LANGFUSE_FLUSH_AT", " 64 ")], Expected::Batch(64_usize), &[])]
#[case::flush_at_above_the_maximum_falls_back(Reading::FlushAt, &[("LANGFUSE_FLUSH_AT", "100001")], Expected::Batch(512_usize), &["LANGFUSE_FLUSH_AT='100001' is not a whole number between 1 and 100000; exporting batches of 512"])]
#[case::flush_at_zero_falls_back(Reading::FlushAt, &[("LANGFUSE_FLUSH_AT", "0")], Expected::Batch(512_usize), &["LANGFUSE_FLUSH_AT='0' is not a whole number between 1 and 100000; exporting batches of 512"])]
#[case::flush_at_empty_value_warns(Reading::FlushAt, &[("LANGFUSE_FLUSH_AT", "")], Expected::Batch(512_usize), &["LANGFUSE_FLUSH_AT='' is not a whole number between 1 and 100000; exporting batches of 512"])]
#[case::flush_at_text_falls_back(Reading::FlushAt, &[("LANGFUSE_FLUSH_AT", "many")], Expected::Batch(512_usize), &["LANGFUSE_FLUSH_AT='many' is not a whole number between 1 and 100000; exporting batches of 512"])]
#[case::sample_rate_default(Reading::SampleRate, &[], Expected::rate(1.0), &[])]
#[case::sample_rate_fraction(Reading::SampleRate, &[("LANGFUSE_SAMPLE_RATE", "0.5")], Expected::rate(0.5), &[])]
#[case::sample_rate_zero(Reading::SampleRate, &[("LANGFUSE_SAMPLE_RATE", "0")], Expected::rate(0.0), &[])]
#[case::sample_rate_one(Reading::SampleRate, &[("LANGFUSE_SAMPLE_RATE", "1")], Expected::rate(1.0), &[])]
#[case::sample_rate_above_one_warns(Reading::SampleRate, &[("LANGFUSE_SAMPLE_RATE", "1.5")], Expected::rate(1.0), &["LANGFUSE_SAMPLE_RATE='1.5' is not a number between 0.0 and 1.0; ignoring it and exporting every trace"])]
#[case::sample_rate_nan_warns(Reading::SampleRate, &[("LANGFUSE_SAMPLE_RATE", "nan")], Expected::rate(1.0), &["LANGFUSE_SAMPLE_RATE='nan' is not a number between 0.0 and 1.0; ignoring it and exporting every trace"])]
#[case::sample_rate_text_warns(Reading::SampleRate, &[("LANGFUSE_SAMPLE_RATE", "half")], Expected::rate(1.0), &["LANGFUSE_SAMPLE_RATE='half' is not a number between 0.0 and 1.0; ignoring it and exporting every trace"])]
#[case::sample_rate_empty_value_warns(Reading::SampleRate, &[("LANGFUSE_SAMPLE_RATE", "")], Expected::rate(1.0), &["LANGFUSE_SAMPLE_RATE='' is not a number between 0.0 and 1.0; ignoring it and exporting every trace"])]
#[case::max_retries_default(Reading::MaxRetries, &[], Expected::Retries(3_u32), &[])]
#[case::max_retries_five(Reading::MaxRetries, &[("LANGFUSE_MAX_RETRIES", "5")], Expected::Retries(5_u32), &[])]
#[case::max_retries_zero_is_allowed(Reading::MaxRetries, &[("LANGFUSE_MAX_RETRIES", "0")], Expected::Retries(0_u32), &[])]
#[case::max_retries_ceiling(Reading::MaxRetries, &[("LANGFUSE_MAX_RETRIES", "1000")], Expected::Retries(1000_u32), &[])]
#[case::max_retries_above_the_ceiling_is_capped(Reading::MaxRetries, &[("LANGFUSE_MAX_RETRIES", "1001")], Expected::Retries(1000_u32), &["LANGFUSE_MAX_RETRIES=1001 is above the ceiling; retrying 1000 times"])]
#[case::max_retries_text_warns(Reading::MaxRetries, &[("LANGFUSE_MAX_RETRIES", "lots")], Expected::Retries(3_u32), &["LANGFUSE_MAX_RETRIES='lots' is not a whole number; retrying 3 times"])]
#[case::max_retries_empty_value_warns(Reading::MaxRetries, &[("LANGFUSE_MAX_RETRIES", "")], Expected::Retries(3_u32), &["LANGFUSE_MAX_RETRIES='' is not a whole number; retrying 3 times"])]
#[case::max_retries_spaces_are_trimmed(Reading::MaxRetries, &[("LANGFUSE_MAX_RETRIES", " 7 ")], Expected::Retries(7_u32), &[])]
#[case::prompt_cache_ttl_unset(Reading::PromptCacheTtl, &[], Expected::rate(60.0), &[])]
#[case::prompt_cache_ttl_zero(Reading::PromptCacheTtl, &[("LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS", "0")], Expected::rate(0.0), &[])]
#[case::prompt_cache_ttl_whole_seconds(Reading::PromptCacheTtl, &[("LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS", "90")], Expected::rate(90.0), &[])]
#[case::prompt_cache_ttl_negative_falls_back(Reading::PromptCacheTtl, &[("LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS", "-1")], Expected::rate(60.0), &["LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS='-1' is not a whole number of seconds at or above 0; caching prompts for 60 s"])]
#[case::prompt_cache_ttl_fraction_falls_back(Reading::PromptCacheTtl, &[("LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS", "60.0")], Expected::rate(60.0), &["LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS='60.0' is not a whole number of seconds at or above 0; caching prompts for 60 s"])]
fn the_number_readers_apply_their_ranges(
    #[case] reading: Reading,
    #[case] variables: &[(&str, &str)],
    #[case] expected: Expected,
    #[case] expected_warnings: &[&str],
) {
    let settings = load(variables);
    let (value, logs) = capture(|| read(reading, &settings));

    assert_eq!(value, expected);
    assert_eq!(warnings(&logs), texts(expected_warnings));
}

#[rstest]
#[case::the_default(&[], 20.0_f64)]
#[case::whole_seconds(&[("LANGFUSE_TIMEOUT", "30")], 30.0_f64)]
#[case::spaces_and_a_fraction(&[("LANGFUSE_TIMEOUT", " 2.5 ")], 2.5_f64)]
#[case::an_exponent(&[("LANGFUSE_TIMEOUT", "1e3")], 1000.0_f64)]
fn timeout_reads_seconds(#[case] variables: &[(&str, &str)], #[case] expected: f64) {
    let settings = load(variables);

    let seconds = timeout_seconds(&settings).expect("a number is a timeout");

    assert_eq!(seconds.to_bits(), expected.to_bits());
}

#[rstest]
#[case::text(&[("LANGFUSE_TIMEOUT", "soon")])]
#[case::an_empty_value(&[("LANGFUSE_TIMEOUT", "")])]
fn a_timeout_that_is_not_a_number_fails(#[case] variables: &[(&str, &str)]) {
    let settings = load(variables);

    let error = timeout_seconds(&settings).expect_err("a non-number fails");

    assert!(matches!(
        &error,
        LangfuseSettingsError::InvalidTimeout { .. }
    ));
    assert!(error.to_string().contains("LANGFUSE_TIMEOUT="));
    assert!(error.to_string().contains("is not a number of seconds"));
}

#[rstest]
#[case::unset(&[])]
#[case::a_whole_number(&[("LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS", "60")])]
#[case::zero(&[("LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS", "0")])]
#[case::a_negative_whole_number(&[("LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS", "-1")])]
#[case::spaces_are_trimmed(&[("LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS", " 90 ")])]
fn a_whole_number_prompt_cache_ttl_passes(#[case] variables: &[(&str, &str)]) {
    let settings = load(variables);

    check_prompt_cache_ttl(&settings).expect("a whole number passes the check");
}

#[rstest]
#[case::text(&[("LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS", "soon")], "soon")]
#[case::a_fraction(&[("LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS", "60.0")], "60.0")]
#[case::an_empty_value(&[("LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS", "")], "")]
#[case::beyond_i64(&[("LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS", "9223372036854775808")], "9223372036854775808")]
fn an_unusable_prompt_cache_ttl_fails_naming_the_variable_and_value(
    #[case] variables: &[(&str, &str)],
    #[case] raw: &str,
) {
    let settings = load(variables);

    let error = check_prompt_cache_ttl(&settings).expect_err("a non-integer fails");

    assert!(matches!(
        &error,
        LangfuseSettingsError::UnusablePromptCacheTtl { .. }
    ));
    assert_eq!(
        error.to_string(),
        format!(
            "LANGFUSE_PROMPT_CACHE_DEFAULT_TTL_SECONDS='{raw}' must be a whole number of seconds"
        )
    );
}
