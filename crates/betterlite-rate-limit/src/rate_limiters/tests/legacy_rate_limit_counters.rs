use std::io;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use betterlite_auth::resolved_key::auth::ParentOtelSpan;
use betterlite_error::rate_limit::{ProxyRateLimitError, RateLimitCategory, RateLimitDimension};
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_redact::structured::Payload;
use chrono::Local;
use rstest::rstest;

use super::support::{
    BatchAnswer, BrokenConsole, CacheCall, FixedClock, FixedSnapshot, LogCapture, RecordingCache,
    RecordingConsole, handler_with,
};
use crate::rate_limiters::legacy_rate_limit_counters::LegacyRateLimitHandler;
use crate::rate_limiters::legacy_rate_limit_counters::clock::{LegacyClock, SystemLegacyClock};
use crate::rate_limiters::legacy_rate_limit_counters::error::LegacyRateLimitError;
use crate::rate_limiters::legacy_rate_limit_counters::limits::{
    DimensionLimits, KeyLimitCheck, RateLimitScope,
};
use crate::rate_limiters::legacy_rate_limit_counters::objects::{CacheObject, CacheObjectKeys};
use crate::rate_limiters::legacy_rate_limit_counters::usage_cache::{
    LegacyCacheValue, LegacyUsageCache, MinuteCounter,
};
use crate::rate_limiters::legacy_rate_limit_counters::verbose::{
    SystemVerboseConsole, VerboseConsole as _,
};

const START_OF_MINUTE: &str = "2026-11-21T12:00:00+00:00";
const CACHE_KEY: &str = "sk-test::2026-11-21-12-00::request_count";
const LIMITS: DimensionLimits = DimensionLimits {
    max_parallel_requests: 5,
    tpm_limit: 20,
    rpm_limit: 10,
};

fn check(
    current: Option<MinuteCounter>,
    dimension_limits: DimensionLimits,
) -> KeyLimitCheck<'static> {
    KeyLimitCheck {
        limits: dimension_limits,
        current,
        request_count_api_key: String::from(CACHE_KEY),
        rate_limit_type: RateLimitScope::Key,
        requested_model: Some("openai/gpt-4o-mini"),
        parent_otel_span: None,
    }
}

fn keys() -> CacheObjectKeys {
    CacheObjectKeys {
        current_global_requests: Some(String::from("global_max_parallel_requests")),
        request_count_api_key: Some(String::from(CACHE_KEY)),
        request_count_api_key_model: None,
        request_count_user_id: Some(String::from("user-1::2026-11-21-12-00::request_count")),
        request_count_team_id: None,
        request_count_end_user_id: Some(String::from(
            "end-user-1::2026-11-21-12-00::request_count",
        )),
    }
}

struct Fixture {
    cache: Arc<RecordingCache>,
    clock: Arc<FixedClock>,
    handler: LegacyRateLimitHandler,
}

fn fixture_with(cache: Arc<RecordingCache>, clock: Arc<FixedClock>) -> Fixture {
    let console = RecordingConsole::new(false);
    let handler = handler_with(&cache, &clock, &console);
    Fixture {
        cache,
        clock,
        handler,
    }
}

fn rejected(error: LegacyRateLimitError) -> ProxyRateLimitError {
    match error {
        LegacyRateLimitError::Rejected(inner) => *inner,
        LegacyRateLimitError::Header(header) => panic!("the retry-after header is valid: {header}"),
    }
}

fn raise_error(
    clock: &Arc<FixedClock>,
    details: Option<&str>,
    dimension: Option<RateLimitDimension>,
    model: Option<&str>,
) -> ProxyRateLimitError {
    let cache = RecordingCache::new();
    let console = RecordingConsole::new(false);
    let handler = handler_with(&cache, clock, &console);
    rejected(handler.raise_rate_limit_error(details, dimension, model))
}

async fn checked(
    fixture: &Fixture,
    current: Option<MinuteCounter>,
    dimension_limits: DimensionLimits,
) -> (
    Result<MinuteCounter, LegacyRateLimitError>,
    Vec<(String, LegacyCacheValue)>,
) {
    let mut updates = Vec::new();
    let result = fixture
        .handler
        .check_key_in_limits(check(current, dimension_limits), &mut updates)
        .await;
    (result, updates)
}

#[test]
fn constructor_stores_the_usage_cache_and_the_clock() {
    let cache = RecordingCache::new();
    let clock = FixedClock::at(START_OF_MINUTE);
    let console = RecordingConsole::new(false);
    let handler = handler_with(&cache, &clock, &console);
    let stored_cache: Arc<dyn LegacyUsageCache> = cache;
    let stored_clock: Arc<dyn LegacyClock> = clock;
    assert!(Arc::ptr_eq(&handler.internal_usage_cache, &stored_cache));
    assert!(Arc::ptr_eq(&handler.clock, &stored_clock));
}

#[rstest]
#[case::off(false, Vec::new())]
#[case::on(true, vec![String::from("statement")])]
fn verbose_logs_at_debug_and_prints_only_when_set_verbose_is_true(
    #[case] set_verbose: bool,
    #[case] printed: Vec<String>,
) {
    let cache = RecordingCache::new();
    let clock = FixedClock::at(START_OF_MINUTE);
    let console = RecordingConsole::new(false);
    let mut handler = handler_with(&cache, &clock, &console);
    handler.set_verbose = set_verbose;
    let capture = LogCapture::start();
    handler.print_verbose("statement");
    let logged = capture.text();
    assert!(
        logged
            .lines()
            .any(|line| line.contains("DEBUG") && line.contains("statement"))
    );
    assert_eq!(console.printed(), printed);
}

#[rstest]
#[case::off(false)]
#[case::on(true)]
fn verbose_ignores_a_failed_console_write(#[case] set_verbose: bool) {
    let cache = RecordingCache::new();
    let clock = FixedClock::at(START_OF_MINUTE);
    let console = RecordingConsole::new(true);
    let mut handler = handler_with(&cache, &clock, &console);
    handler.set_verbose = set_verbose;
    let capture = LogCapture::start();
    handler.print_verbose("statement");
    assert_eq!(capture.text().contains("could not print"), set_verbose);
    assert!(console.printed().is_empty());
}

#[test]
fn verbose_console_writes_the_statement_and_reports_a_broken_console() {
    let stdout_console = SystemVerboseConsole {
        writer: io::stdout(),
    };
    assert!(stdout_console.print_line("legacy verbose statement"));
    let broken = SystemVerboseConsole {
        writer: BrokenConsole,
    };
    assert!(!broken.print_line("legacy verbose statement"));
}

#[test]
fn system_clock_reads_the_wall_clock_now() {
    let read = SystemLegacyClock.now().timestamp();
    let now = Local::now().fixed_offset().timestamp();
    assert!(read <= now.saturating_add(300));
    assert!(read >= now.saturating_sub(300));
}

#[rstest]
#[case::start(START_OF_MINUTE, 60.0_f64)]
#[case::middle("2026-11-21T12:00:30.250000+00:00", 29.75_f64)]
#[case::end("2026-11-21T12:00:59.999950+00:00", 0.00005_f64)]
#[test]
fn retry_interval_counts_to_the_next_minute(#[case] at: &str, #[case] expected: f64) {
    let fixture = fixture_with(RecordingCache::new(), FixedClock::at(at));
    assert_eq!(
        fixture.handler.time_to_next_minute().to_bits(),
        expected.to_bits()
    );
    assert_eq!(fixture.clock.reads(), 1);
}

#[test]
fn error_raising_defaults_to_concurrent_requests() {
    let clock = FixedClock::at(START_OF_MINUTE);
    let error = raise_error(&clock, None, None, None);
    assert_eq!(
        error.detail,
        Payload::String(String::from("Max parallel request limit reached"))
    );
    assert_eq!(error.message, "Max parallel request limit reached");
    assert_eq!(
        error.rate_limit_type,
        Some(RateLimitDimension::ConcurrentRequests)
    );
    assert_eq!(error.category, RateLimitCategory::LitellmRateLimit);
    assert_eq!(error.model, "");
    assert_eq!(error.llm_provider, "litellm_proxy");
    let headers = error
        .headers
        .as_ref()
        .expect("the error carries a retry-after header");
    assert_eq!(headers["retry-after"], "60.0");
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers()["retry-after"], "60.0");
}

#[test]
fn error_raising_appends_the_additional_details_and_keeps_the_type() {
    let clock = FixedClock::at(START_OF_MINUTE);
    let error = raise_error(
        &clock,
        Some("Hit Global Limit: Limit=5, current: 5"),
        Some(RateLimitDimension::Tokens),
        None,
    );
    assert_eq!(
        error.message,
        "Max parallel request limit reached Hit Global Limit: Limit=5, current: 5"
    );
    assert_eq!(error.rate_limit_type, Some(RateLimitDimension::Tokens));
}

#[rstest]
#[case::whole_minute(START_OF_MINUTE, "60.0")]
#[case::micro_second("2026-11-21T12:00:59.999950+00:00", "5e-05")]
#[test]
fn error_raising_writes_the_retry_after_like_python(#[case] at: &str, #[case] expected: &str) {
    let clock = FixedClock::at(at);
    let error = raise_error(&clock, None, None, None);
    let headers = error
        .headers
        .as_ref()
        .expect("the error carries a retry-after header");
    assert_eq!(headers["retry-after"], expected);
}

#[rstest]
#[case::native(Some("openai/gpt-4o-mini"), "gpt-4o-mini", "openai")]
#[case::unknown(Some("unknown-model"), "unknown-model", "litellm_proxy")]
#[case::absent(None, "", "litellm_proxy")]
#[test]
fn error_provider_resolves_or_falls_back(
    #[case] model: Option<&str>,
    #[case] name: &str,
    #[case] provider: &str,
) {
    let clock = FixedClock::at(START_OF_MINUTE);
    let error = raise_error(&clock, None, None, model);
    assert_eq!(error.model, name);
    assert_eq!(error.llm_provider, provider);
}

#[test]
fn error_provider_resolves_through_a_router_alias() {
    let cache = RecordingCache::new();
    let clock = FixedClock::at(START_OF_MINUTE);
    let console = RecordingConsole::new(false);
    let mut handler = handler_with(&cache, &clock, &console);
    let row = Json::Object(JsonObject::from([
        (
            "model_name".to_owned(),
            Json::Text(String::from("my-alias")),
        ),
        (
            "litellm_params".to_owned(),
            Json::Object(JsonObject::from([(
                "model".to_owned(),
                Json::Text(String::from("openai/gpt-4o-mini")),
            )])),
        ),
    ]));
    handler.router = Some(FixedSnapshot::new(vec![row]));
    let error = rejected(handler.raise_rate_limit_error(None, None, Some("my-alias")));
    assert_eq!(error.model, "gpt-4o-mini");
    assert_eq!(error.llm_provider, "openai");
}

#[tokio::test]
async fn dimension_precedence_raises_on_concurrent_requests_first() {
    let fixture = fixture_with(RecordingCache::new(), FixedClock::at(START_OF_MINUTE));
    let (result, updates) = checked(
        &fixture,
        Some(MinuteCounter {
            current_requests: 5,
            current_tpm: 20,
            current_rpm: 10,
        }),
        LIMITS,
    )
    .await;
    let error = rejected(result.expect_err("over the parallel request limit"));
    assert_eq!(
        error.rate_limit_type,
        Some(RateLimitDimension::ConcurrentRequests)
    );
    assert_eq!(
        error.message,
        "LiteLLM Rate Limit Handler for rate limit type = key. Crossed TPM / RPM / Max Parallel Request Limit. current rpm: 10, rpm limit: 10, current tpm: 20, tpm limit: 20, current max_parallel_requests: 5, max_parallel_requests: 5"
    );
    assert_eq!(error.model, "gpt-4o-mini");
    assert_eq!(error.llm_provider, "openai");
    assert!(updates.is_empty());
    assert!(fixture.cache.calls().is_empty());
}

#[rstest]
#[case::inside(MinuteCounter { current_requests: 4, current_tpm: 19, current_rpm: 9 }, None)]
#[case::parallel_at_the_limit(
    MinuteCounter { current_requests: 5, current_tpm: 0, current_rpm: 0 },
    Some(RateLimitDimension::ConcurrentRequests)
)]
#[case::tokens_at_the_limit(
    MinuteCounter { current_requests: 1, current_tpm: 20, current_rpm: 10 },
    Some(RateLimitDimension::Tokens)
)]
#[case::tokens_at_the_limit_alone(
    MinuteCounter { current_requests: 1, current_tpm: 20, current_rpm: 5 },
    Some(RateLimitDimension::Tokens)
)]
#[case::requests_at_the_rpm_limit(
    MinuteCounter { current_requests: 1, current_tpm: 5, current_rpm: 10 },
    Some(RateLimitDimension::Requests)
)]
#[tokio::test]
async fn dimension_precedence_treats_each_limit_as_exclusive(
    #[case] current: MinuteCounter,
    #[case] dimension: Option<RateLimitDimension>,
) {
    let fixture = fixture_with(RecordingCache::new(), FixedClock::at(START_OF_MINUTE));
    let capture = LogCapture::start();
    let (result, _updates) = checked(&fixture, Some(current), LIMITS).await;
    assert!(
        capture
            .text()
            .contains("Current Usage of key in this minute: requests ")
    );
    match dimension {
        None => assert_eq!(
            result.expect("inside every limit"),
            MinuteCounter {
                current_requests: 5,
                current_tpm: 19,
                current_rpm: 10,
            }
        ),
        Some(expected) => {
            let error = rejected(result.expect_err("at a limit"));
            assert_eq!(error.rate_limit_type, Some(expected));
        }
    }
}

#[tokio::test]
async fn dimension_precedence_initializes_a_missing_counter() {
    let fixture = fixture_with(RecordingCache::new(), FixedClock::at(START_OF_MINUTE));
    let capture = LogCapture::start();
    let (result, updates) = checked(&fixture, None, LIMITS).await;
    assert!(
        capture
            .text()
            .contains("Current Usage of key in this minute: none")
    );
    assert_eq!(
        result.expect("no limit is zero"),
        MinuteCounter {
            current_requests: 1,
            current_tpm: 0,
            current_rpm: 1,
        }
    );
    assert_eq!(
        updates,
        vec![(
            String::from(CACHE_KEY),
            LegacyCacheValue::Counter(MinuteCounter {
                current_requests: 1,
                current_tpm: 0,
                current_rpm: 1,
            }),
        )]
    );
}

#[rstest]
#[case::every_limit(
    DimensionLimits { max_parallel_requests: 0, tpm_limit: 0, rpm_limit: 0 },
    RateLimitDimension::ConcurrentRequests
)]
#[case::parallel_only(
    DimensionLimits { max_parallel_requests: 0, tpm_limit: 5, rpm_limit: 5 },
    RateLimitDimension::ConcurrentRequests
)]
#[case::tokens_and_rpm(
    DimensionLimits { max_parallel_requests: 5, tpm_limit: 0, rpm_limit: 0 },
    RateLimitDimension::Tokens
)]
#[case::tokens_with_rpm_set(
    DimensionLimits { max_parallel_requests: 5, tpm_limit: 0, rpm_limit: 10 },
    RateLimitDimension::Tokens
)]
#[case::rpm_only(
    DimensionLimits { max_parallel_requests: 5, tpm_limit: 10, rpm_limit: 0 },
    RateLimitDimension::Requests
)]
#[tokio::test]
async fn all_zero_limits_raise_with_the_first_zero_dimension(
    #[case] dimension_limits: DimensionLimits,
    #[case] expected: RateLimitDimension,
) {
    let fixture = fixture_with(RecordingCache::new(), FixedClock::at(START_OF_MINUTE));
    let (result, updates) = checked(&fixture, None, dimension_limits).await;
    let error = rejected(result.expect_err("a zero limit blocks every request"));
    assert_eq!(error.rate_limit_type, Some(expected));
    assert_eq!(error.model, "gpt-4o-mini");
    assert_eq!(error.llm_provider, "openai");
    assert!(updates.is_empty());
    assert!(fixture.cache.calls().is_empty());
}

#[tokio::test]
async fn all_zero_limits_name_every_limit_in_the_detail() {
    let fixture = fixture_with(RecordingCache::new(), FixedClock::at(START_OF_MINUTE));
    let (result, _updates) = checked(
        &fixture,
        None,
        DimensionLimits {
            max_parallel_requests: 0,
            tpm_limit: 0,
            rpm_limit: 0,
        },
    )
    .await;
    let error = rejected(result.expect_err("every limit is zero"));
    assert_eq!(
        error.message,
        "Max parallel request limit reached Crossed TPM / RPM / Max Parallel Request Limit. Hit limit for key. Current limits: max_parallel_requests: 0, tpm_limit: 0, rpm_limit: 0"
    );
}

#[tokio::test]
async fn cache_updates_writes_the_whole_list_with_ttl_and_local_only() {
    let fixture = fixture_with(RecordingCache::new(), FixedClock::at(START_OF_MINUTE));
    let earlier = (
        String::from("earlier::2026-11-21-12-00::request_count"),
        LegacyCacheValue::Count(3),
    );
    let mut updates = vec![earlier.clone()];
    let current = MinuteCounter {
        current_requests: 3,
        current_tpm: 10,
        current_rpm: 5,
    };
    let request = check(Some(current), LIMITS);
    let new_val = fixture
        .handler
        .check_key_in_limits(request, &mut updates)
        .await
        .expect("under every limit");
    let counted = MinuteCounter {
        current_requests: 4,
        current_tpm: 10,
        current_rpm: 6,
    };
    assert_eq!(new_val, counted);
    let expected = vec![
        earlier,
        (String::from(CACHE_KEY), LegacyCacheValue::Counter(counted)),
    ];
    assert_eq!(updates, expected);
    assert_eq!(
        fixture.cache.calls(),
        vec![CacheCall::BatchSet {
            entries: expected,
            ttl: 60,
            local_only: true,
            span: false,
        }]
    );
}

#[tokio::test]
async fn cache_updates_does_not_write_when_the_limit_raises() {
    let fixture = fixture_with(RecordingCache::new(), FixedClock::at(START_OF_MINUTE));
    let earlier = (
        String::from("earlier::2026-11-21-12-00::request_count"),
        LegacyCacheValue::Count(3),
    );
    let mut updates = vec![earlier.clone()];
    let result = fixture
        .handler
        .check_key_in_limits(
            check(
                Some(MinuteCounter {
                    current_requests: 5,
                    current_tpm: 20,
                    current_rpm: 10,
                }),
                DimensionLimits {
                    max_parallel_requests: 5,
                    tpm_limit: 20,
                    rpm_limit: 10,
                },
            ),
            &mut updates,
        )
        .await;
    let error = rejected(result.expect_err("over the parallel request limit"));
    assert_eq!(
        error.rate_limit_type,
        Some(RateLimitDimension::ConcurrentRequests)
    );
    assert_eq!(updates, vec![earlier]);
    assert!(fixture.cache.calls().is_empty());
}

#[tokio::test]
async fn cache_updates_keeps_going_when_the_write_fails() {
    let fixture = fixture_with(
        RecordingCache::with_failing_writes(),
        FixedClock::at(START_OF_MINUTE),
    );
    let capture = LogCapture::start();
    let (result, updates) = checked(
        &fixture,
        Some(MinuteCounter {
            current_requests: 3,
            current_tpm: 10,
            current_rpm: 5,
        }),
        LIMITS,
    )
    .await;
    assert_eq!(
        result.expect("the write failure does not refuse the request"),
        MinuteCounter {
            current_requests: 4,
            current_tpm: 10,
            current_rpm: 6,
        }
    );
    assert_eq!(updates.len(), 1);
    assert!(capture.text().contains("async add_cache"));
}

#[tokio::test]
async fn cache_updates_forwards_the_parent_span() {
    let fixture = fixture_with(RecordingCache::new(), FixedClock::at(START_OF_MINUTE));
    let span = ParentOtelSpan::Opaque;
    let current = MinuteCounter {
        current_requests: 3,
        current_tpm: 10,
        current_rpm: 5,
    };
    let request = KeyLimitCheck {
        limits: LIMITS,
        current: Some(current),
        request_count_api_key: String::from(CACHE_KEY),
        rate_limit_type: RateLimitScope::Key,
        requested_model: None,
        parent_otel_span: Some(&span),
    };
    let mut updates = Vec::new();
    let new_val = fixture
        .handler
        .check_key_in_limits(request, &mut updates)
        .await
        .expect("under every limit");
    let counted = MinuteCounter {
        current_requests: 4,
        current_tpm: 10,
        current_rpm: 6,
    };
    assert_eq!(new_val, counted);
    assert_eq!(
        fixture.cache.calls(),
        vec![CacheCall::BatchSet {
            entries: updates,
            ttl: 60,
            local_only: true,
            span: true,
        }]
    );
}

const APIKEY_COUNTER: MinuteCounter = MinuteCounter {
    current_requests: 1,
    current_tpm: 2,
    current_rpm: 3,
};
const MODEL_COUNTER: MinuteCounter = MinuteCounter {
    current_requests: 4,
    current_tpm: 5,
    current_rpm: 6,
};
const TEAM_COUNTER: MinuteCounter = MinuteCounter {
    current_requests: 7,
    current_tpm: 8,
    current_rpm: 9,
};
const USER_COUNTER: MinuteCounter = MinuteCounter {
    current_requests: 10,
    current_tpm: 11,
    current_rpm: 12,
};

#[tokio::test]
async fn batch_object_maps_each_result_to_its_field() {
    let values = vec![
        Some(LegacyCacheValue::Count(7)),
        Some(LegacyCacheValue::Counter(APIKEY_COUNTER)),
        Some(LegacyCacheValue::Counter(MODEL_COUNTER)),
        None,
        Some(LegacyCacheValue::Counter(TEAM_COUNTER)),
        Some(LegacyCacheValue::Counter(USER_COUNTER)),
    ];
    let fixture = fixture_with(
        RecordingCache::answering(BatchAnswer::Values(values)),
        FixedClock::at(START_OF_MINUTE),
    );
    let object = fixture.handler.get_all_cache_objects(keys(), None).await;
    let expected = CacheObject {
        current_global_requests: Some(LegacyCacheValue::Count(7)),
        request_count_api_key: Some(LegacyCacheValue::Counter(APIKEY_COUNTER)),
        request_count_api_key_model: Some(LegacyCacheValue::Counter(MODEL_COUNTER)),
        request_count_user_id: None,
        request_count_team_id: Some(LegacyCacheValue::Counter(TEAM_COUNTER)),
        request_count_end_user_id: Some(LegacyCacheValue::Counter(USER_COUNTER)),
    };
    assert_eq!(object, expected);
}

#[test]
fn batch_object_lists_the_keys_in_field_order() {
    assert_eq!(
        keys().into_list(),
        [
            Some(String::from("global_max_parallel_requests")),
            Some(String::from(CACHE_KEY)),
            None,
            Some(String::from("user-1::2026-11-21-12-00::request_count")),
            None,
            Some(String::from("end-user-1::2026-11-21-12-00::request_count")),
        ]
    );
}

#[tokio::test]
async fn batch_object_forwards_the_keys_and_the_parent_span() {
    let fixture = fixture_with(RecordingCache::new(), FixedClock::at(START_OF_MINUTE));
    let span = ParentOtelSpan::Opaque;
    let object = fixture
        .handler
        .get_all_cache_objects(keys(), Some(&span))
        .await;
    assert_eq!(object, CacheObject::default());
    assert_eq!(
        fixture.cache.calls(),
        vec![CacheCall::BatchGet {
            keys: keys().into_list().to_vec(),
            span: true,
        }]
    );
}

#[rstest]
#[case::failed(BatchAnswer::Failed, "async_batch_get_cache")]
#[case::short(
    BatchAnswer::Values(vec![Some(LegacyCacheValue::Count(1))]),
    "wrong length"
)]
#[tokio::test]
async fn batch_object_answers_all_none_without_a_full_read(
    #[case] answer: BatchAnswer,
    #[case] logged: &str,
) {
    let fixture = fixture_with(
        RecordingCache::answering(answer),
        FixedClock::at(START_OF_MINUTE),
    );
    let capture = LogCapture::start();
    let object = fixture.handler.get_all_cache_objects(keys(), None).await;
    assert_eq!(object, CacheObject::default());
    assert!(capture.text().contains(logged));
}

#[rstest]
#[case::padded_and_formatted("2026-01-02T03:04:05+00:00", "2026-01-02-03-04")]
#[case::clock_offset("2026-01-01T00:30:00+05:30", "2026-01-01-00-30")]
#[test]
fn precise_minute_formats_the_local_wall_clock(#[case] at: &str, #[case] expected: &str) {
    let fixture = fixture_with(RecordingCache::new(), FixedClock::at(at));
    assert_eq!(fixture.handler.precise_minute(), expected);
    assert_eq!(fixture.clock.reads(), 1);
}
