#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::iter::repeat_with;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::langfuse_settings::Settings;
use opentelemetry::KeyValue;
use opentelemetry::trace::{
    Link, Span as _, SpanContext, SpanId, TraceFlags, TraceId, TraceState, Tracer as _,
};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::trace::{SpanData, SpanExporter};
use rstest::rstest;
use secrecy::SecretString;
use tokio::runtime::{Builder, Handle};
use tokio::sync::Semaphore;
use tokio::task::yield_now;
use tokio::time::{Instant, advance, sleep};
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::langfuse_exporter::error::LangfuseExporterError;
use crate::langfuse_observations::requested_ids::RequestedIds;

use super::cache::{CacheState, TRACING};
use super::{
    CHANNEL_RETIRE_GRACE_SECONDS, ChannelSpec, DEFAULT_FLUSH_TIMEOUT_MILLIS, LangfuseChannelError,
    LangfuseChannelRequest, LangfuseTracing, MAX_QUEUE_SIZE, SPAN_LIMITS, TRACER_NAME, UNSET,
    acquire_langfuse_tracing, build_langfuse_tracing, flush_langfuse_tracing,
    release_langfuse_tracing,
};

const PUBLIC_KEY: &str = "pk-test";
const BASE_URL: &str = "https://langfuse.test";
const SECRET: &str = "sk-test";

fn lookup(variables: &[(&str, &str)]) -> BTreeMap<String, String> {
    variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn settings(variables: &[(&str, &str)]) -> Settings {
    ConfigLoader {
        lookup: lookup(variables),
    }
    .load("")
    .expect("the langfuse settings load")
    .sections
    .langfuse_settings
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn request(public_key: &str) -> LangfuseChannelRequest {
    LangfuseChannelRequest {
        public_key: public_key.to_owned(),
        secret_key: secret(SECRET),
        base_url: BASE_URL.to_owned(),
        environment: None,
        release: None,
        flush_interval: 1,
        mock_mode: true,
    }
}

fn spec() -> ChannelSpec<'static> {
    ChannelSpec {
        environment: None,
        release: None,
        sample_rate: 1.0,
        flush_interval_millis: 60_000,
        flush_at: 512,
    }
}

fn schedule(flush_interval_millis: u64, flush_at: usize) -> ChannelSpec<'static> {
    ChannelSpec {
        environment: None,
        release: None,
        sample_rate: 1.0,
        flush_interval_millis,
        flush_at,
    }
}

fn acquire_counting(
    request: &LangfuseChannelRequest,
    builds: &Arc<AtomicUsize>,
) -> LangfuseTracing {
    let counter = Arc::clone(builds);
    acquire_langfuse_tracing(
        &settings(&[]),
        &lookup(&[]),
        &Handle::current(),
        request,
        move || {
            counter.fetch_add(1, Ordering::AcqRel);
            Ok(Recording::default())
        },
    )
    .expect("the channel is acquired")
}

fn acquire_mock(request: &LangfuseChannelRequest) -> LangfuseTracing {
    acquire_langfuse_tracing(
        &settings(&[]),
        &lookup(&[]),
        &Handle::current(),
        request,
        || Ok(Recording::default()),
    )
    .expect("the mock channel is acquired")
}

fn acquire_with<E>(request: &LangfuseChannelRequest, exporter: E) -> LangfuseTracing
where
    E: SpanExporter + 'static,
{
    acquire_langfuse_tracing(
        &settings(&[]),
        &lookup(&[]),
        &Handle::current(),
        request,
        || Ok(exporter),
    )
    .expect("the live channel is acquired")
}

fn lease_of<'state>(
    state: &'state CacheState,
    tracing: &LangfuseTracing,
) -> Option<&'state super::cache::Lease> {
    state
        .leases
        .values()
        .find(|lease| Arc::ptr_eq(&lease.tracing.ledger.exporter, &tracing.ledger.exporter))
}

fn cached(tracing: &LangfuseTracing) -> Option<u32> {
    let state = TRACING.lock().unwrap_or_else(PoisonError::into_inner);
    lease_of(&state, tracing).map(|lease| lease.holders)
}

fn retire_id(tracing: &LangfuseTracing) -> Option<u64> {
    let state = TRACING.lock().unwrap_or_else(PoisonError::into_inner);
    let lease = lease_of(&state, tracing)?;
    let id = lease.retire.as_ref().map(|retire| retire.id);
    drop(state);
    id
}

#[derive(Clone, Debug, Default)]
struct Recording {
    spans: Arc<Mutex<Vec<SpanData>>>,
    resource: Arc<Mutex<Option<Resource>>>,
    shut_down: Arc<AtomicUsize>,
}

impl Recording {
    fn spans(&self) -> Vec<SpanData> {
        self.spans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn resource(&self) -> Option<Resource> {
        self.resource
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn span_count(&self) -> usize {
        self.spans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    fn shut_down(&self) -> bool {
        self.shut_down.load(Ordering::Acquire) > 0
    }
}

impl SpanExporter for Recording {
    fn export(&self, batch: Vec<SpanData>) -> impl Future<Output = OTelSdkResult> + Send {
        self.spans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(batch);
        ready(Ok(()))
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        self.shut_down.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    fn set_resource(&mut self, resource: &Resource) {
        *self.resource.lock().unwrap_or_else(PoisonError::into_inner) = Some(resource.clone());
    }
}

#[derive(Clone, Debug, Default)]
struct Failing {
    failures: Arc<AtomicUsize>,
}

impl Failing {
    fn failure_count(&self) -> usize {
        self.failures.load(Ordering::Acquire)
    }
}

impl SpanExporter for Failing {
    fn export(&self, _batch: Vec<SpanData>) -> impl Future<Output = OTelSdkResult> + Send {
        self.failures.fetch_add(1, Ordering::AcqRel);
        ready(Err(OTelSdkError::InternalFailure(
            "the destination refused the batch".to_owned(),
        )))
    }
}

#[derive(Clone, Debug)]
struct Gated {
    gate: Arc<Semaphore>,
}

impl SpanExporter for Gated {
    async fn export(&self, _batch: Vec<SpanData>) -> OTelSdkResult {
        let _permit = self.gate.acquire().await.expect("the gate stays open");
        Ok(())
    }
}

fn end_span(tracing: &LangfuseTracing, name: &'static str) {
    let mut span = tracing.tracer.span_builder(name).start(&tracing.tracer);
    span.end();
}

async fn wait_until<F>(check: F)
where
    F: Fn() -> bool,
{
    for _ in 0..600_u32 {
        if check() {
            return;
        }
        sleep(Duration::from_millis(5)).await;
    }
    assert!(check(), "the condition did not hold in time");
}

fn linked_context(index: usize) -> SpanContext {
    let mut trace = [0_u8; 16];
    trace[15] = u8::try_from(index).expect("a link index fits a byte");
    SpanContext::new(
        TraceId::from_bytes(trace),
        SpanId::from_bytes([2; 8]),
        TraceFlags::SAMPLED,
        true,
        TraceState::NONE,
    )
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

fn capture<T, F>(level: Option<LevelFilter>, run: F) -> (T, Logs)
where
    F: FnOnce() -> T,
{
    let logs = Logs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let value = match level {
        Some(filter) => with_default(subscriber.with(filter), run),
        None => with_default(subscriber, run),
    };
    (value, logs)
}

#[test]
fn the_channel_constants_match_the_sdk_contract() {
    assert_eq!(TRACER_NAME, "langfuse-sdk");
    assert_eq!(CHANNEL_RETIRE_GRACE_SECONDS.to_bits(), 60.0_f64.to_bits());
    assert_eq!(MAX_QUEUE_SIZE, 100_000);
    assert_eq!(DEFAULT_FLUSH_TIMEOUT_MILLIS, 30_000);
    assert_eq!(SPAN_LIMITS.max_attributes_per_span, UNSET);
    assert_eq!(SPAN_LIMITS.max_events_per_span, 128);
    assert_eq!(SPAN_LIMITS.max_links_per_span, 128);
    assert_eq!(SPAN_LIMITS.max_attributes_per_event, 128);
    assert_eq!(SPAN_LIMITS.max_attributes_per_link, 128);
}

#[test]
fn the_default_retirement_grace_is_sixty_seconds() {
    let grace = Duration::try_from_secs_f64(CHANNEL_RETIRE_GRACE_SECONDS)
        .expect("the constant is a usable number of seconds");
    assert_eq!(grace, Duration::from_secs(60));
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
#[case::true_warns("true", Some("also disables the langfuse callback"))]
#[case::capitalized_and_padded_warns(" TRUE ", Some("also disables the langfuse callback"))]
#[case::one_warns("1", Some("also disables the langfuse callback"))]
#[case::empty_does_not_warn("", None)]
#[case::false_does_not_warn("false", None)]
#[case::unusable_value_warns_about_the_parse("maybe", Some("is not a bool"))]
#[tokio::test]
async fn otel_sdk_disabled_warns_and_still_builds_the_provider(
    #[case] value: &str,
    #[case] expected: Option<&str>,
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
    assert_eq!(exporter.span_count(), 1);
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
    release_langfuse_tracing(&tracing, Some(Duration::ZERO)).await;
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
    release_langfuse_tracing(&tracing, Some(Duration::ZERO)).await;
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
    release_langfuse_tracing(&first, Some(Duration::ZERO)).await;
    assert_eq!(cached(&second), Some(1));
    release_langfuse_tracing(&second, Some(Duration::ZERO)).await;
}

#[tokio::test(start_paused = true)]
async fn the_last_release_retires_the_channel_after_the_grace_period() {
    let tracing = acquire_mock(&request("pk-retire"));
    release_langfuse_tracing(&tracing, None).await;
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
    let first = acquire_mock(&request);
    release_langfuse_tracing(&first, None).await;
    yield_now().await;
    advance(Duration::from_secs(30)).await;
    let second = acquire_mock(&request);
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
    release_langfuse_tracing(&tracing, Some(Duration::from_millis(20))).await;
    wait_until(|| exporter.shut_down()).await;
    assert_eq!(cached(&tracing), None);
}

#[tokio::test]
async fn a_stale_retirement_does_not_remove_a_reacquired_channel() {
    let request = request("pk-stale-retire");
    let tracing = acquire_mock(&request);
    let spec = super::key::channel_spec(&settings(&[]), &request);
    let key = super::key::tracing_key(&request, &spec);
    super::cache::retire_unless_reacquired(key, 99, tracing.clone(), Duration::ZERO).await;
    assert!(cached(&tracing).is_some());
}

#[tokio::test(start_paused = true)]
async fn a_release_while_retirement_is_pending_changes_nothing() {
    let tracing = acquire_mock(&request("pk-double-release"));
    release_langfuse_tracing(&tracing, None).await;
    yield_now().await;
    let pending = retire_id(&tracing);
    assert!(pending.is_some());
    release_langfuse_tracing(&tracing, None).await;
    assert_eq!(retire_id(&tracing), pending);
    assert_eq!(cached(&tracing), Some(0));
}

#[tokio::test]
async fn releasing_without_grace_shuts_the_channel_down_and_forgets_it() {
    let request = request("pk-immediate");
    let tracing = acquire_mock(&request);
    release_langfuse_tracing(&tracing, Some(Duration::ZERO)).await;
    assert_eq!(cached(&tracing), None);
    assert!(!tracing.flush(1_000).await);
    let again = acquire_mock(&request);
    assert!(!Arc::ptr_eq(
        &tracing.ledger.exporter,
        &again.ledger.exporter
    ));
    release_langfuse_tracing(&again, Some(Duration::ZERO)).await;
}

#[tokio::test]
async fn releasing_a_channel_that_is_not_cached_leaves_it_running() {
    let tracing = build_langfuse_tracing(
        Recording::default(),
        &Handle::current(),
        &spec(),
        &lookup(&[]),
    );
    release_langfuse_tracing(&tracing, Some(Duration::ZERO)).await;
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
    release_langfuse_tracing(&first, Some(Duration::ZERO)).await;
    release_langfuse_tracing(&second, Some(Duration::ZERO)).await;
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

#[tokio::test]
async fn mock_mode_discards_spans_and_never_builds_the_live_exporter() {
    let builds = Arc::new(AtomicUsize::new(0));
    let builds_for_factory = Arc::clone(&builds);
    let tracing = acquire_langfuse_tracing(
        &settings(&[]),
        &lookup(&[]),
        &Handle::current(),
        &request("pk-mock"),
        move || {
            builds_for_factory.fetch_add(1, Ordering::AcqRel);
            Ok(Recording::default())
        },
    )
    .expect("the mock channel is acquired");
    assert_eq!(builds.load(Ordering::Acquire), 0);
    end_span(&tracing, "discarded");
    assert!(tracing.flush(5_000).await);
    release_langfuse_tracing(&tracing, Some(Duration::ZERO)).await;
}

#[tokio::test]
async fn a_rejected_batch_makes_the_flush_report_failure() {
    let exporter = Failing::default();
    let tracing =
        build_langfuse_tracing(exporter.clone(), &Handle::current(), &spec(), &lookup(&[]));
    end_span(&tracing, "rejected");
    assert!(!tracing.flush(5_000).await);
    assert_eq!(tracing.ledger.failed_batches.load(Ordering::Acquire), 1);
    tracing.shutdown();
}

#[tokio::test]
async fn a_failure_before_the_flush_does_not_taint_the_next_one() {
    let exporter = Failing::default();
    let tracing = build_langfuse_tracing(
        exporter.clone(),
        &Handle::current(),
        &schedule(10, 512),
        &lookup(&[]),
    );
    end_span(&tracing, "scheduled-failure");
    wait_until(|| exporter.failure_count() == 1).await;
    assert!(tracing.flush(5_000).await);
    assert_eq!(tracing.ledger.failed_batches.load(Ordering::Acquire), 1);
    tracing.shutdown();
}

#[tokio::test]
async fn a_flush_that_misses_its_deadline_reports_failure() {
    let gate = Arc::new(Semaphore::new(0));
    let exporter = Gated {
        gate: Arc::clone(&gate),
    };
    let tracing = build_langfuse_tracing(exporter, &Handle::current(), &spec(), &lookup(&[]));
    end_span(&tracing, "blocked");
    let started = Instant::now();
    assert!(!tracing.flush(50).await);
    assert!(started.elapsed() < Duration::from_secs(2));
    gate.add_permits(16);
    sleep(Duration::from_millis(50)).await;
    tracing.shutdown();
}

#[tokio::test]
async fn a_full_batch_is_exported_without_a_flush() {
    let exporter = Recording::default();
    let tracing = build_langfuse_tracing(
        exporter.clone(),
        &Handle::current(),
        &schedule(60_000, 1),
        &lookup(&[]),
    );
    end_span(&tracing, "one-of-one");
    let spans = Arc::clone(&exporter.spans);
    wait_until(|| {
        !spans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    })
    .await;
    assert_eq!(exporter.span_count(), 1);
    tracing.shutdown();
}

#[tokio::test]
async fn the_schedule_delay_comes_from_the_flush_interval() {
    let exporter = Recording::default();
    let tracing = build_langfuse_tracing(
        exporter.clone(),
        &Handle::current(),
        &schedule(20, 512),
        &lookup(&[]),
    );
    end_span(&tracing, "on-the-schedule");
    let spans = Arc::clone(&exporter.spans);
    wait_until(|| {
        !spans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    })
    .await;
    assert!(tracing.flush(5_000).await);
    tracing.shutdown();
}

#[tokio::test]
async fn the_provider_hands_out_the_requested_span_ids() {
    let exporter = Recording::default();
    let tracing =
        build_langfuse_tracing(exporter.clone(), &Handle::current(), &spec(), &lookup(&[]));
    let trace_id = TraceId::from_hex("0af7651916cd43dd8448eb211c80319c").expect("a trace id");
    let span_id = SpanId::from_hex("b7ad6b7169203331").expect("a span id");
    let requested = RequestedIds::enter(Some(trace_id), Some(span_id));
    let mut span = tracing
        .tracer
        .span_builder("requested")
        .start(&tracing.tracer);
    span.end();
    drop(requested);
    assert!(tracing.flush(5_000).await);
    let spans = exporter.spans();
    assert_eq!(spans[0].span_context.trace_id(), trace_id);
    assert_eq!(spans[0].span_context.span_id(), span_id);
    tracing.shutdown();
}

#[tokio::test]
async fn a_zero_sample_rate_exports_no_spans() {
    let exporter = Recording::default();
    let unsampled = ChannelSpec {
        environment: None,
        release: None,
        sample_rate: 0.0,
        flush_interval_millis: 60_000,
        flush_at: 512,
    };
    let tracing = build_langfuse_tracing(
        exporter.clone(),
        &Handle::current(),
        &unsampled,
        &lookup(&[]),
    );
    end_span(&tracing, "dropped");
    assert!(tracing.flush(5_000).await);
    assert!(exporter.spans().is_empty());
    tracing.shutdown();
}

#[rstest]
#[case::sampled("22222222222222222222222222222222", 1)]
#[case::dropped("11111111111111111111111111111111", 0)]
#[tokio::test]
async fn a_partial_sample_rate_keeps_the_hash_sampler_in_charge(
    #[case] trace: &str,
    #[case] exported: usize,
) {
    let exporter = Recording::default();
    let sampling = ChannelSpec {
        environment: None,
        release: None,
        sample_rate: 0.5,
        flush_interval_millis: 60_000,
        flush_at: 512,
    };
    let tracing = build_langfuse_tracing(
        exporter.clone(),
        &Handle::current(),
        &sampling,
        &lookup(&[]),
    );
    let trace_id = TraceId::from_hex(trace).expect("a trace id");
    let requested = RequestedIds::enter(Some(trace_id), None);
    let mut span = tracing
        .tracer
        .span_builder("sampled")
        .start(&tracing.tracer);
    span.end();
    drop(requested);
    assert!(tracing.flush(5_000).await);
    assert_eq!(exporter.span_count(), exported);
    tracing.shutdown();
}

#[tokio::test]
async fn an_exporter_that_cannot_be_built_fails_the_acquisition() {
    let mut request = request("pk-broken-exporter");
    request.mock_mode = false;
    let Err(error) = acquire_langfuse_tracing(
        &settings(&[]),
        &lookup(&[]),
        &Handle::current(),
        &request,
        || {
            Err::<Recording, _>(LangfuseExporterError::Endpoint {
                endpoint: "not a url".to_owned(),
                source: url::ParseError::RelativeUrlWithoutBase,
            })
        },
    ) else {
        panic!("a broken exporter must fail the acquisition");
    };
    assert!(matches!(error, LangfuseChannelError::Exporter(_)));
    let state = TRACING.lock().unwrap_or_else(PoisonError::into_inner);
    assert!(state.leases.is_empty());
    drop(state);
}

#[tokio::test]
async fn flushing_all_channels_returns_true_when_every_queue_drains() {
    let first = acquire_mock(&request("pk-flush-all-a"));
    let second = acquire_mock(&request("pk-flush-all-b"));
    end_span(&first, "first");
    end_span(&second, "second");
    assert!(flush_langfuse_tracing(5_000).await);
    release_langfuse_tracing(&first, Some(Duration::ZERO)).await;
    release_langfuse_tracing(&second, Some(Duration::ZERO)).await;
}

#[tokio::test]
async fn flushing_all_channels_returns_false_when_one_misses_the_deadline() {
    let fast_recording = Recording::default();
    let mut fast_request = request("pk-flush-fast");
    fast_request.mock_mode = false;
    let fast = acquire_with(&fast_request, fast_recording.clone());
    let gate = Arc::new(Semaphore::new(0));
    let mut slow_request = request("pk-flush-slow");
    slow_request.mock_mode = false;
    let slow = acquire_with(
        &slow_request,
        Gated {
            gate: Arc::clone(&gate),
        },
    );
    end_span(&fast, "fast");
    end_span(&slow, "slow");
    let started = Instant::now();
    assert!(!flush_langfuse_tracing(300).await);
    assert!(started.elapsed() < Duration::from_secs(3));
    let exported = Arc::clone(&fast_recording.spans);
    wait_until(|| {
        !exported
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    })
    .await;
    gate.add_permits(16);
    sleep(Duration::from_millis(50)).await;
    release_langfuse_tracing(&fast, Some(Duration::ZERO)).await;
    release_langfuse_tracing(&slow, Some(Duration::ZERO)).await;
}

#[tokio::test]
async fn flushing_all_channels_reports_a_channel_whose_delivery_failed() {
    let healthy = acquire_mock(&request("pk-flush-mixed-a"));
    let mut failing_request = request("pk-flush-mixed-b");
    failing_request.mock_mode = false;
    let failing = acquire_with(&failing_request, Failing::default());
    end_span(&healthy, "healthy");
    end_span(&failing, "failing");
    assert!(!flush_langfuse_tracing(5_000).await);
    release_langfuse_tracing(&healthy, Some(Duration::ZERO)).await;
    release_langfuse_tracing(&failing, Some(Duration::ZERO)).await;
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
