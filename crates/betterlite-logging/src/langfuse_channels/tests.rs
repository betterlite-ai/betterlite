#![cfg(test)]

use std::collections::BTreeMap;
use std::iter::repeat_with;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use betterlite_config::sections::langfuse_settings::Settings;
use opentelemetry::KeyValue;
use opentelemetry::trace::{Link, Span as _, Tracer as _};
use rstest::rstest;
use tokio::runtime::{Builder, Handle};
use tokio::task::yield_now;
use tokio::time::advance;
use tracing::Level;
use tracing_subscriber::filter::LevelFilter;

use super::support::{
    PUBLIC_KEY, Recording, SECRET, acquire_counting, acquire_with, cached, capture, end_span,
    linked_context, lookup, request, retire_id, secret, settings, spec, wait_until,
};
use super::{
    CHANNEL_RETIRE_GRACE, ChannelSpec, DEFAULT_FLUSH_TIMEOUT_MILLIS, LangfuseChannelError,
    LangfuseChannelRequest, LangfuseTracing, MAX_QUEUE_SIZE, SPAN_LIMITS, TRACER_NAME, UNSET,
    acquire_langfuse_tracing, build_langfuse_tracing, release_langfuse_tracing,
};

#[test]
fn the_channel_constants_match_the_sdk_contract() {
    assert_eq!(TRACER_NAME, "langfuse-sdk");
    assert_eq!(MAX_QUEUE_SIZE, 100_000);
    assert_eq!(DEFAULT_FLUSH_TIMEOUT_MILLIS, 30_000);
    assert_eq!(SPAN_LIMITS.max_attributes_per_span, UNSET);
    assert_eq!(SPAN_LIMITS.max_events_per_span, 128);
    assert_eq!(SPAN_LIMITS.max_links_per_span, 128);
    assert_eq!(SPAN_LIMITS.max_attributes_per_event, 128);
    assert_eq!(SPAN_LIMITS.max_attributes_per_link, 128);
}

#[rstest]
#[case::one_second(1, 1_000)]
#[case::zero(0, 0)]
#[case::half_a_minute(30, 30_000)]
fn the_flush_interval_in_seconds_becomes_milliseconds(#[case] seconds: u64, #[case] millis: u64) {
    let mut request = request("pk-interval");
    request.flush_interval = seconds;
    let spec = super::key::channel_spec(&settings(&[]), &request);
    assert_eq!(spec.flush_interval_millis, millis);
}

#[test]
fn the_default_retirement_grace_is_sixty_seconds() {
    assert_eq!(CHANNEL_RETIRE_GRACE, Duration::from_secs(60));
}

#[tokio::test]
async fn the_tracer_carries_the_langfuse_sdk_scope_name() {
    let exporter = Recording::default();
    let tracing =
        build_langfuse_tracing(exporter.clone(), &Handle::current(), &spec(), &lookup(&[]));
    end_span(&tracing, "scope");
    assert!(tracing.flush(5_000).await);
    let spans = exporter.spans();
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].instrumentation_scope.name(), TRACER_NAME);
}

#[tokio::test]
async fn the_span_limits_cap_events_and_links_but_not_span_attributes() {
    let exporter = Recording::default();
    let tracing =
        build_langfuse_tracing(exporter.clone(), &Handle::current(), &spec(), &lookup(&[]));
    let attributes: Vec<KeyValue> = (0..200_usize)
        .map(|index| KeyValue::new(format!("key-{index}"), i64::try_from(index).expect("fits")))
        .collect();
    let crowded: Vec<KeyValue> = (0..200_usize)
        .map(|index| {
            KeyValue::new(
                format!("event-key-{index}"),
                i64::try_from(index).expect("fits"),
            )
        })
        .collect();
    let span_links: Vec<Link> = (0..200_usize)
        .map(|index| Link::new(linked_context(index), crowded.clone(), 0))
        .collect();
    let mut span = tracing
        .tracer
        .span_builder("limits")
        .with_attributes(attributes)
        .with_links(span_links)
        .start(&tracing.tracer);
    span.add_event("crowded", crowded);
    (0..200_usize).for_each(|_| span.add_event("event", Vec::new()));
    span.end();
    assert!(tracing.flush(5_000).await);
    let recorded = exporter.spans();
    let span_record = &recorded[0];
    assert_eq!(span_record.attributes.len(), 200);
    assert_eq!(span_record.events.len(), 128);
    assert_eq!(span_record.links.len(), 128);
    assert_eq!(span_record.events[0].attributes.len(), 128);
    assert_eq!(span_record.links[0].attributes.len(), 128);
}

#[rstest]
#[case::both_present(Some("production"), Some("v1.2.3"), 2)]
#[case::empty_values_are_written(Some(""), Some(""), 2)]
#[case::absent_values_are_omitted(None, None, 0)]
#[tokio::test]
async fn the_exporter_is_given_the_environment_and_release_resource(
    #[case] environment: Option<&str>,
    #[case] release: Option<&str>,
    #[case] attributes: usize,
) {
    let exporter = Recording::default();
    let built = ChannelSpec {
        environment,
        release,
        sample_rate: 1.0,
        flush_interval_millis: 60_000,
        flush_at: 512,
    };
    let channel =
        build_langfuse_tracing(exporter.clone(), &Handle::current(), &built, &lookup(&[]));
    let resource = exporter
        .resource()
        .expect("the exporter received a resource");
    let recorded: BTreeMap<String, String> = resource
        .iter()
        .map(|(key, value)| (key.as_str().to_owned(), value.as_str().into_owned()))
        .collect();
    assert_eq!(recorded.len(), attributes);
    assert_eq!(
        recorded.get("langfuse.environment").map(String::as_str),
        environment
    );
    assert_eq!(
        recorded.get("langfuse.release").map(String::as_str),
        release
    );
    channel.shutdown();
}

#[rstest]
#[case::true_warns("true", Some("also disables the langfuse callback"), 0)]
#[case::capitalized_and_padded_warns(" TRUE ", Some("also disables the langfuse callback"), 0)]
#[case::one_warns("1", Some("also disables the langfuse callback"), 0)]
#[case::empty_does_not_warn("", None, 1)]
#[case::false_does_not_warn("false", None, 1)]
#[case::unusable_value_warns_about_the_parse("maybe", Some("is not a bool"), 1)]
#[tokio::test]
async fn otel_sdk_disabled_warns_and_still_builds_the_provider(
    #[case] value: &str,
    #[case] expected: Option<&str>,
    #[case] exported: usize,
) {
    let exporter = Recording::default();
    let environment = lookup(&[("OTEL_SDK_DISABLED", value)]);
    let (tracing, logs) = capture(None, || {
        build_langfuse_tracing(exporter.clone(), &Handle::current(), &spec(), &environment)
    });
    let warnings = logs.messages(Level::WARN);
    match expected {
        Some(fragment) => assert!(
            warnings.iter().any(|message| message.contains(fragment)),
            "no warning contained {fragment:?}: {warnings:?}"
        ),
        None => assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}"),
    }
    end_span(&tracing, "after-disabled");
    assert!(tracing.flush(5_000).await);
    assert_eq!(exporter.span_count(), exported);
}

#[tokio::test]
async fn langfuse_debug_raises_the_debug_logging_when_the_filter_allows_it() {
    let (tracing, logs) = capture(None, || {
        acquire_langfuse_tracing(
            &settings(&[("LANGFUSE_DEBUG", "true")]),
            &lookup(&[]),
            &Handle::current(),
            &request(PUBLIC_KEY),
            || Ok(Recording::default()),
        )
        .expect("the channel is acquired")
    });
    assert!(
        logs.messages(Level::DEBUG)
            .iter()
            .any(|message| message.contains("LANGFUSE_DEBUG"))
    );
    release_langfuse_tracing(&tracing, Duration::ZERO).await;
}

#[tokio::test]
async fn langfuse_debug_reports_when_the_filter_hides_the_output() {
    let (tracing, logs) = capture(Some(LevelFilter::INFO), || {
        acquire_langfuse_tracing(
            &settings(&[("LANGFUSE_DEBUG", "true")]),
            &lookup(&[]),
            &Handle::current(),
            &request(PUBLIC_KEY),
            || Ok(Recording::default()),
        )
        .expect("the channel is acquired")
    });
    assert!(
        logs.messages(Level::INFO)
            .iter()
            .any(|message| message.contains("LANGFUSE_DEBUG"))
    );
    release_langfuse_tracing(&tracing, Duration::ZERO).await;
}

#[tokio::test]
async fn an_unusable_langfuse_debug_value_fails_the_acquisition() {
    let Err(error) = acquire_langfuse_tracing(
        &settings(&[("LANGFUSE_DEBUG", "maybe")]),
        &lookup(&[]),
        &Handle::current(),
        &request(PUBLIC_KEY),
        || Ok(Recording::default()),
    ) else {
        panic!("an unusable flag must fail the acquisition");
    };
    assert!(matches!(error, LangfuseChannelError::Settings(_)));
    assert!(error.to_string().contains("LANGFUSE_DEBUG"));
}

#[tokio::test]
async fn the_same_credentials_hand_out_one_channel() {
    let builds = Arc::new(AtomicUsize::new(0));
    let mut request = request("pk-shared");
    request.mock_mode = false;
    let first = acquire_counting(&request, &builds);
    let second = acquire_counting(&request, &builds);
    assert!(Arc::ptr_eq(&first.ledger.exporter, &second.ledger.exporter));
    assert_eq!(builds.load(Ordering::Acquire), 1);
    assert_eq!(cached(&first), Some(2));
    release_langfuse_tracing(&first, Duration::ZERO).await;
    assert_eq!(cached(&second), Some(1));
    release_langfuse_tracing(&second, Duration::ZERO).await;
}

#[tokio::test(start_paused = true)]
async fn the_last_release_retires_the_channel_after_the_grace_period() {
    let tracing = acquire_with(&request("pk-retire"), Recording::default());
    release_langfuse_tracing(&tracing, CHANNEL_RETIRE_GRACE).await;
    yield_now().await;
    assert!(cached(&tracing).is_some());
    advance(Duration::from_secs(59)).await;
    assert!(cached(&tracing).is_some());
    advance(Duration::from_secs(1)).await;
    yield_now().await;
    assert_eq!(cached(&tracing), None);
}

#[tokio::test(start_paused = true)]
async fn reacquisition_before_the_deadline_cancels_the_retirement() {
    let request = request("pk-reacquire");
    let first = acquire_with(&request, Recording::default());
    release_langfuse_tracing(&first, CHANNEL_RETIRE_GRACE).await;
    yield_now().await;
    advance(Duration::from_secs(30)).await;
    let second = acquire_with(&request, Recording::default());
    assert!(Arc::ptr_eq(&first.ledger.exporter, &second.ledger.exporter));
    assert_eq!(cached(&first), Some(1));
    assert_eq!(retire_id(&first), None);
    advance(Duration::from_secs(60)).await;
    yield_now().await;
    assert_eq!(cached(&first), Some(1));
}

#[tokio::test]
async fn the_retirement_timer_shuts_the_provider_down() {
    let exporter = Recording::default();
    let mut request = request("pk-retire-shutdown");
    request.mock_mode = false;
    let tracing = acquire_with(&request, exporter.clone());
    release_langfuse_tracing(&tracing, Duration::from_millis(20)).await;
    wait_until(|| exporter.shut_down()).await;
    assert_eq!(cached(&tracing), None);
}

#[tokio::test]
async fn a_stale_retirement_does_not_remove_a_reacquired_channel() {
    let request = request("pk-stale-retire");
    let tracing = acquire_with(&request, Recording::default());
    let spec = super::key::channel_spec(&settings(&[]), &request);
    let key = super::key::tracing_key(&request, &spec);
    super::cache::retire_unless_reacquired(key, 99, tracing.clone(), Duration::ZERO).await;
    assert!(cached(&tracing).is_some());
}

#[tokio::test(start_paused = true)]
async fn a_release_while_retirement_is_pending_changes_nothing() {
    let tracing = acquire_with(&request("pk-double-release"), Recording::default());
    release_langfuse_tracing(&tracing, CHANNEL_RETIRE_GRACE).await;
    yield_now().await;
    let pending = retire_id(&tracing);
    assert!(pending.is_some());
    release_langfuse_tracing(&tracing, CHANNEL_RETIRE_GRACE).await;
    assert_eq!(retire_id(&tracing), pending);
    assert_eq!(cached(&tracing), Some(0));
}

#[tokio::test]
async fn releasing_without_grace_shuts_the_channel_down_and_forgets_it() {
    let request = request("pk-immediate");
    let tracing = acquire_with(&request, Recording::default());
    release_langfuse_tracing(&tracing, Duration::ZERO).await;
    assert_eq!(cached(&tracing), None);
    assert!(!tracing.flush(1_000).await);
    let again = acquire_with(&request, Recording::default());
    assert!(!Arc::ptr_eq(
        &tracing.ledger.exporter,
        &again.ledger.exporter
    ));
    release_langfuse_tracing(&again, Duration::ZERO).await;
}

#[tokio::test]
async fn releasing_a_channel_that_is_not_cached_leaves_it_running() {
    let tracing = build_langfuse_tracing(
        Recording::default(),
        &Handle::current(),
        &spec(),
        &lookup(&[]),
    );
    release_langfuse_tracing(&tracing, Duration::ZERO).await;
    end_span(&tracing, "still-running");
    assert!(tracing.flush(5_000).await);
    tracing.shutdown();
}

enum Change {
    PublicKey,
    SecretKey,
    BaseUrl,
    Environment,
    Release,
    SampleRate,
    FlushAt,
    FlushInterval,
    MockMode,
}

fn changed(base: &LangfuseChannelRequest, change: &Change) -> LangfuseChannelRequest {
    let mut changed = LangfuseChannelRequest {
        public_key: base.public_key.clone(),
        secret_key: secret(SECRET),
        base_url: base.base_url.clone(),
        environment: base.environment.clone(),
        release: base.release.clone(),
        flush_interval: base.flush_interval,
        mock_mode: base.mock_mode,
    };
    match change {
        Change::PublicKey => changed.public_key.push_str("-other"),
        Change::SecretKey => changed.secret_key = secret("sk-other"),
        Change::BaseUrl => changed.base_url.push_str("/other"),
        Change::Environment => changed.environment = Some("production".to_owned()),
        Change::Release => changed.release = Some("v2".to_owned()),
        Change::FlushInterval => changed.flush_interval = changed.flush_interval.saturating_add(1),
        Change::MockMode => changed.mock_mode = !changed.mock_mode,
        Change::SampleRate | Change::FlushAt => {}
    }
    changed
}

fn settings_for(change: &Change) -> Settings {
    match change {
        Change::SampleRate => settings(&[("LANGFUSE_SAMPLE_RATE", "0.5")]),
        Change::FlushAt => settings(&[("LANGFUSE_FLUSH_AT", "64")]),
        Change::PublicKey
        | Change::SecretKey
        | Change::BaseUrl
        | Change::Environment
        | Change::Release
        | Change::FlushInterval
        | Change::MockMode => settings(&[]),
    }
}

#[rstest]
#[case::public_key(Change::PublicKey)]
#[case::secret_key(Change::SecretKey)]
#[case::base_url(Change::BaseUrl)]
#[case::environment(Change::Environment)]
#[case::release(Change::Release)]
#[case::sample_rate(Change::SampleRate)]
#[case::flush_at(Change::FlushAt)]
#[case::flush_interval(Change::FlushInterval)]
#[case::mock_mode(Change::MockMode)]
#[tokio::test]
async fn every_acquire_parameter_separates_the_channels(#[case] change: Change) {
    let base = request("pk-key");
    let changed = changed(&base, &change);
    let environment = lookup(&[]);
    let first = acquire_langfuse_tracing(
        &settings_for(&change),
        &environment,
        &Handle::current(),
        &base,
        || Ok(Recording::default()),
    )
    .expect("the first channel");
    let second = acquire_langfuse_tracing(
        &settings(&[]),
        &environment,
        &Handle::current(),
        &changed,
        || Ok(Recording::default()),
    )
    .expect("the changed channel");
    assert!(!Arc::ptr_eq(
        &first.ledger.exporter,
        &second.ledger.exporter
    ));
    release_langfuse_tracing(&first, Duration::ZERO).await;
    release_langfuse_tracing(&second, Duration::ZERO).await;
}

#[test]
fn concurrent_acquisitions_hand_out_one_channel() {
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the runtime builds");
    let environment_map = lookup(&[]);
    let settings_value = settings(&[]);
    let request_value = request("pk-concurrent");
    let channels: Vec<LangfuseTracing> = thread::scope(|scope| {
        let handles: Vec<_> = repeat_with(|| {
            let environment = &environment_map;
            let settings = &settings_value;
            let request = &request_value;
            let handle = runtime.handle();
            scope.spawn(move || {
                let factory = || Ok(Recording::default());
                acquire_langfuse_tracing(settings, environment, handle, request, factory)
                    .expect("the channel is acquired")
            })
        })
        .take(8)
        .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("the acquiring thread finishes"))
            .collect()
    });
    let first = channels.first().expect("a channel was acquired");
    assert!(
        channels
            .iter()
            .all(|channel| Arc::ptr_eq(&first.ledger.exporter, &channel.ledger.exporter))
    );
    assert_eq!(cached(first), Some(8));
}

#[test]
fn the_secret_digest_is_taken_from_the_secret() {
    let request = request("pk-digest");
    let first = tracing_key_for(&request, SECRET);
    let second = tracing_key_for(&request, "sk-other");
    assert_ne!(first, second);
}

fn tracing_key_for(request: &LangfuseChannelRequest, secret_text: &str) -> super::TracingKey {
    let copy = LangfuseChannelRequest {
        public_key: request.public_key.clone(),
        secret_key: secret(secret_text),
        base_url: request.base_url.clone(),
        environment: request.environment.clone(),
        release: request.release.clone(),
        flush_interval: request.flush_interval,
        mock_mode: request.mock_mode,
    };
    let spec = super::key::channel_spec(&settings(&[]), &copy);
    super::key::tracing_key(&copy, &spec)
}
