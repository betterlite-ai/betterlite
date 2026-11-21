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
    RecordingConsole, handler_deps,
};
use crate::rate_limiters::legacy_rate_limit_counters::LegacyRateLimitHandler;
use crate::rate_limiters::legacy_rate_limit_counters::clock::{LegacyClock, SystemLegacyClock};
use crate::rate_limiters::legacy_rate_limit_counters::error::LegacyRateLimitError;
use crate::rate_limiters::legacy_rate_limit_counters::limits::{
    DimensionLimits, KeyLimitCheck, RateLimitScope,
};
use crate::rate_limiters::legacy_rate_limit_counters::objects::{CacheObject, CacheObjectKeys};
use crate::rate_limiters::legacy_rate_limit_counters::retry_after::float_text;
use crate::rate_limiters::legacy_rate_limit_counters::usage_cache::{
    LegacyCacheValue, LegacyUsageCache, MinuteCounter,
};
use crate::rate_limiters::legacy_rate_limit_counters::verbose::{
    SystemVerboseConsole, VerboseConsole as _,
};

const START_OF_MINUTE: &str = "2026-11-21T12:00:00+00:00";
const CACHE_KEY: &str = "sk-test::2026-11-21-12-00::request_count";

const fn counter(current_requests: i64, current_tpm: i64, current_rpm: i64) -> MinuteCounter {
    MinuteCounter {
        current_requests,
        current_tpm,
        current_rpm,
    }
}

const fn limits(max_parallel_requests: i64, tpm_limit: i64, rpm_limit: i64) -> DimensionLimits {
    DimensionLimits {
        max_parallel_requests,
        tpm_limit,
        rpm_limit,
    }
}

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

fn fixture() -> Fixture {
    fixture_with(RecordingCache::new(), FixedClock::at(START_OF_MINUTE))
}

fn fixture_with(cache: Arc<RecordingCache>, clock: Arc<FixedClock>) -> Fixture {
    let console = RecordingConsole::new(false);
    let handler = LegacyRateLimitHandler::new(handler_deps(&cache, &clock, &console));
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
    let handler = LegacyRateLimitHandler::new(handler_deps(&cache, clock, &console));
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
    let handler = LegacyRateLimitHandler::new(handler_deps(&cache, &clock, &console));
    let stored_cache: Arc<dyn LegacyUsageCache> = Arc::clone(&cache);
    let stored_clock: Arc<dyn LegacyClock> = Arc::clone(&clock);
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
    let mut deps = handler_deps(&cache, &clock, &console);
    deps.set_verbose = set_verbose;
    let handler = LegacyRateLimitHandler::new(deps);
    let capture = LogCapture::start();
    handler.print_verbose("statement");
    assert!(capture.text().contains("DEBUG"));
    assert_eq!(console.printed(), printed);
}

#[rstest]
#[case::off(false)]
#[case::on(true)]
fn verbose_ignores_a_failed_console_write(#[case] set_verbose: bool) {
    let cache = RecordingCache::new();
    let clock = FixedClock::at(START_OF_MINUTE);
    let console = RecordingConsole::new(true);
    let mut deps = handler_deps(&cache, &clock, &console);
    deps.set_verbose = set_verbose;
    let handler = LegacyRateLimitHandler::new(deps);
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
    let mut deps = handler_deps(&cache, &clock, &console);
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
    deps.router = Some(FixedSnapshot::new(vec![row]));
    let handler = LegacyRateLimitHandler::new(deps);
    let error = rejected(handler.raise_rate_limit_error(None, None, Some("my-alias")));
    assert_eq!(error.model, "gpt-4o-mini");
    assert_eq!(error.llm_provider, "openai");
}

#[tokio::test]
async fn dimension_precedence_raises_on_concurrent_requests_first() {
    let fixture = fixture();
    let (result, updates) = checked(&fixture, Some(counter(5, 20, 10)), limits(5, 20, 10)).await;
    let error = rejected(result.expect_err("over the parallel request limit"));
    assert_eq!(
        error.rate_limit_type,
        Some(RateLimitDimension::ConcurrentRequests)
    );
    assert_eq!(
        error.message,
        "LiteLLM Rate Limit Handler for rate limit type = key. Crossed TPM / RPM / Max Parallel Request Limit. current rpm: 10, rpm limit: 10, current tpm: 20, tpm limit: 20, current max_parallel_requests: 5, max_parallel_requests: 5"
    );
    assert!(updates.is_empty());
    assert!(fixture.cache.calls().is_empty());
}

#[rstest]
#[case::inside(counter(4, 19, 9), None)]
#[case::parallel_at_the_limit(counter(5, 0, 0), Some(RateLimitDimension::ConcurrentRequests))]
#[case::tokens_at_the_limit(counter(1, 20, 10), Some(RateLimitDimension::Tokens))]
#[case::tokens_at_the_limit_alone(counter(1, 20, 5), Some(RateLimitDimension::Tokens))]
#[case::requests_at_the_rpm_limit(counter(1, 5, 10), Some(RateLimitDimension::Requests))]
#[tokio::test]
async fn dimension_precedence_treats_each_limit_as_exclusive(
    #[case] current: MinuteCounter,
    #[case] dimension: Option<RateLimitDimension>,
) {
    let fixture = fixture();
    let (result, _updates) = checked(&fixture, Some(current), limits(5, 20, 10)).await;
    match dimension {
        None => assert_eq!(result.expect("inside every limit"), counter(5, 19, 10)),
        Some(expected) => {
            let error = rejected(result.expect_err("at a limit"));
            assert_eq!(error.rate_limit_type, Some(expected));
        }
    }
}

#[tokio::test]
async fn dimension_precedence_initializes_a_missing_counter() {
    let fixture = fixture();
    let (result, updates) = checked(&fixture, None, limits(5, 20, 10)).await;
    assert_eq!(result.expect("no limit is zero"), counter(1, 0, 1));
    assert_eq!(
        updates,
        vec![(
            String::from(CACHE_KEY),
            LegacyCacheValue::Counter(counter(1, 0, 1)),
        )]
    );
}

#[rstest]
#[case::every_limit(limits(0, 0, 0), RateLimitDimension::ConcurrentRequests)]
#[case::parallel_only(limits(0, 5, 5), RateLimitDimension::ConcurrentRequests)]
#[case::tokens_and_rpm(limits(5, 0, 0), RateLimitDimension::Tokens)]
#[case::tokens_with_rpm_set(limits(5, 0, 10), RateLimitDimension::Tokens)]
#[case::rpm_only(limits(5, 10, 0), RateLimitDimension::Requests)]
#[tokio::test]
async fn all_zero_limits_raise_with_the_first_zero_dimension(
    #[case] dimension_limits: DimensionLimits,
    #[case] expected: RateLimitDimension,
) {
    let fixture = fixture();
    let (result, updates) = checked(&fixture, None, dimension_limits).await;
    let error = rejected(result.expect_err("a zero limit blocks every request"));
    assert_eq!(error.rate_limit_type, Some(expected));
    assert!(updates.is_empty());
    assert!(fixture.cache.calls().is_empty());
}

#[tokio::test]
async fn all_zero_limits_name_every_limit_in_the_detail() {
    let fixture = fixture();
    let (result, _updates) = checked(&fixture, None, limits(0, 0, 0)).await;
    let error = rejected(result.expect_err("every limit is zero"));
    assert_eq!(
        error.message,
        "Max parallel request limit reached Crossed TPM / RPM / Max Parallel Request Limit. Hit limit for key. Current limits: max_parallel_requests: 0, tpm_limit: 0, rpm_limit: 0"
    );
}

#[tokio::test]
async fn cache_updates_writes_the_whole_list_with_ttl_and_local_only() {
    let fixture = fixture();
    let earlier = (
        String::from("earlier::2026-11-21-12-00::request_count"),
        LegacyCacheValue::Count(3),
    );
    let mut updates = vec![earlier.clone()];
    let new_val = fixture
        .handler
        .check_key_in_limits(
            check(Some(counter(3, 10, 5)), limits(5, 20, 10)),
            &mut updates,
        )
        .await
        .expect("under every limit");
    assert_eq!(new_val, counter(4, 10, 6));
    let expected = vec![
        earlier,
        (
            String::from(CACHE_KEY),
            LegacyCacheValue::Counter(counter(4, 10, 6)),
        ),
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
    let fixture = fixture();
    let earlier = (
        String::from("earlier::2026-11-21-12-00::request_count"),
        LegacyCacheValue::Count(3),
    );
    let mut updates = vec![earlier.clone()];
    let result = fixture
        .handler
        .check_key_in_limits(
            check(Some(counter(5, 20, 10)), limits(5, 20, 10)),
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
    let (result, updates) = checked(&fixture, Some(counter(3, 10, 5)), limits(5, 20, 10)).await;
    assert_eq!(
        result.expect("the write failure does not refuse the request"),
        counter(4, 10, 6)
    );
    assert_eq!(updates.len(), 1);
    assert!(capture.text().contains("async add_cache"));
}

#[tokio::test]
async fn cache_updates_forwards_the_parent_span() {
    let fixture = fixture();
    let span = ParentOtelSpan::Opaque;
    let request = KeyLimitCheck {
        limits: limits(5, 20, 10),
        current: Some(counter(3, 10, 5)),
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
    assert_eq!(new_val, counter(4, 10, 6));
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

#[tokio::test]
async fn batch_object_maps_each_result_to_its_field() {
    let values = vec![
        Some(LegacyCacheValue::Count(7)),
        Some(LegacyCacheValue::Counter(counter(1, 2, 3))),
        Some(LegacyCacheValue::Counter(counter(4, 5, 6))),
        None,
        Some(LegacyCacheValue::Counter(counter(7, 8, 9))),
        Some(LegacyCacheValue::Counter(counter(10, 11, 12))),
    ];
    let fixture = fixture_with(
        RecordingCache::answering(BatchAnswer::Values(values)),
        FixedClock::at(START_OF_MINUTE),
    );
    let object = fixture.handler.get_all_cache_objects(keys(), None).await;
    let expected = CacheObject {
        current_global_requests: Some(LegacyCacheValue::Count(7)),
        request_count_api_key: Some(LegacyCacheValue::Counter(counter(1, 2, 3))),
        request_count_api_key_model: Some(LegacyCacheValue::Counter(counter(4, 5, 6))),
        request_count_user_id: None,
        request_count_team_id: Some(LegacyCacheValue::Counter(counter(7, 8, 9))),
        request_count_end_user_id: Some(LegacyCacheValue::Counter(counter(10, 11, 12))),
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
    let fixture = fixture();
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

#[rstest]
#[case::minute(60.0_f64, "60.0")]
#[case::zero(0.0_f64, "0.0")]
#[case::fraction(0.5_f64, "0.5")]
#[case::fixed_edge(0.0001_f64, "0.0001")]
#[case::micro(0.00005_f64, "5e-05")]
#[case::one_micro(1e-06_f64, "1e-06")]
fn float_text_matches_python_str(#[case] value: f64, #[case] expected: &str) {
    assert_eq!(float_text(value), expected);
}
