#![cfg(test)]
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use http::HeaderMap;
use opentelemetry::trace::{Span as _, Tracer as _};
use opentelemetry::{Context, KeyValue};
use opentelemetry_http::{Bytes, HttpClient, HttpError, Request, Response};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{
    InMemorySpanExporter, SdkTracerProvider, Span, SpanData, SpanProcessor,
};
use rstest::rstest;

use super::{
    CacheKey, CacheSettings, DynamicTracerCache, SHUTDOWN_THREAD_NAME, legacy_tracer_name,
};
use crate::otel::legacy_exporters::build::LegacyExporterBuilder;
use crate::otel::legacy_exporters::error::LegacyExporterError;
use crate::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig, Signals};
use crate::otel::plumbing::exporters::{ExporterError, HttpClients};

#[derive(Debug)]
struct NoServer;

#[async_trait::async_trait]
impl HttpClient for NoServer {
    async fn send_bytes(&self, _request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        Err(HttpError::from("no HTTP client is wired into this test"))
    }
}

fn clients() -> HttpClients {
    let shared: Arc<dyn HttpClient> = Arc::new(NoServer);
    HttpClients {
        traces: Arc::clone(&shared),
        metrics: Arc::clone(&shared),
        logs: shared,
    }
}

fn config(
    exporter: Exporter,
    endpoint: Option<&str>,
    headers: Option<&str>,
) -> OpenTelemetryConfig {
    OpenTelemetryConfig {
        exporter,
        endpoint: endpoint.map(str::to_owned),
        headers: headers.map(str::to_owned),
        signals: Signals::default(),
        service_name: "litellm".to_owned(),
        deployment_environment: "production".to_owned(),
        model_id: "litellm".to_owned(),
        ignore_context_propagation: false,
        skip_set_global: false,
        capture_message_content: None,
        semconv_stability_opt_in: BTreeSet::new(),
        baggage_team_metadata_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        attributes: None,
    }
}

fn cache_with(
    settings_config: OpenTelemetryConfig,
    max_providers: usize,
    shared: InMemorySpanExporter,
    http: HttpClients,
) -> DynamicTracerCache {
    DynamicTracerCache::new(
        CacheSettings {
            config: settings_config,
            callback_name: None,
            resource: Resource::builder_empty().build(),
            tracer_name: "litellm".to_owned(),
            max_providers,
        },
        LegacyExporterBuilder::new(http, shared),
    )
}

fn cache(settings_config: OpenTelemetryConfig, max_providers: usize) -> DynamicTracerCache {
    cache_with(
        settings_config,
        max_providers,
        InMemorySpanExporter::default(),
        clients(),
    )
}

#[derive(Debug, Clone)]
struct Posted {
    uri: String,
    headers: HeaderMap,
    body: Vec<u8>,
}

#[derive(Debug, Default)]
struct Recorder(Mutex<Vec<Posted>>);

impl Recorder {
    fn take(&self) -> Vec<Posted> {
        let mut posted = self.0.lock().expect("request lock");
        mem::take(&mut *posted)
    }
}

#[async_trait::async_trait]
impl HttpClient for Recorder {
    async fn send_bytes(&self, request: Request<Bytes>) -> Result<Response<Bytes>, HttpError> {
        let posted = Posted {
            uri: request.uri().to_string(),
            headers: request.headers().clone(),
            body: request.body().to_vec(),
        };
        self.0.lock().expect("request lock").push(posted);
        Ok(Response::new(Bytes::new()))
    }
}

fn recording_clients(recorder: &Arc<Recorder>) -> HttpClients {
    let concrete: Arc<Recorder> = Arc::clone(recorder);
    let shared: Arc<dyn HttpClient> = concrete;
    HttpClients {
        traces: Arc::clone(&shared),
        metrics: Arc::clone(&shared),
        logs: shared,
    }
}

fn header<'posted>(posted: &'posted Posted, name: &str) -> Option<&'posted str> {
    posted.headers.get(name)?.to_str().ok()
}

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn config_key(text: &str) -> CacheKey {
    CacheKey::Config {
        exporter: text.to_owned(),
        endpoint: None,
        headers: None,
    }
}

fn otlp(endpoint: &str) -> OpenTelemetryConfig {
    config(
        Exporter::Named("otlp_http".to_owned()),
        Some(endpoint),
        None,
    )
}

#[derive(Debug)]
struct Record {
    key: String,
    thread: String,
}

#[derive(Debug)]
struct ShutdownRecorder {
    key: String,
    sender: SyncSender<Record>,
}

impl SpanProcessor for ShutdownRecorder {
    fn on_start(&self, _span: &mut Span, _cx: &Context) {}

    fn on_end(&self, _span: SpanData) {}

    fn force_flush(&self) -> OTelSdkResult {
        Ok(())
    }

    fn shutdown_with_timeout(&self, _timeout: Duration) -> OTelSdkResult {
        let thread = thread::current().name().unwrap_or("unnamed").to_owned();
        drop(self.sender.try_send(Record {
            key: self.key.clone(),
            thread,
        }));
        Ok(())
    }
}

fn provider(key: &str, sender: &SyncSender<Record>) -> SdkTracerProvider {
    SdkTracerProvider::builder()
        .with_span_processor(ShutdownRecorder {
            key: key.to_owned(),
            sender: sender.clone(),
        })
        .build()
}

fn drain(receiver: &Receiver<Record>) -> Vec<Record> {
    let mut records = Vec::new();
    loop {
        match receiver.try_recv() {
            Ok(record) => records.push(record),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => return records,
        }
    }
}

fn shut_down_on_own_thread(records: &[Record]) -> Vec<&str> {
    records
        .iter()
        .filter(|record| record.thread == SHUTDOWN_THREAD_NAME)
        .map(|record| record.key.as_str())
        .collect()
}

fn build_error() -> LegacyExporterError {
    LegacyExporterError::Exporter(ExporterError::UnknownKind("console".to_owned()))
}

#[rstest]
#[case::unset(None, "litellm")]
#[case::set(Some("proxy-tracer"), "proxy-tracer")]
#[case::empty(Some(""), "litellm")]
fn legacy_tracer_name_reads_the_environment(#[case] value: Option<&str>, #[case] expected: &str) {
    let env = value.map_or_else(BTreeMap::new, |text| map(&[("OTEL_TRACER_NAME", text)]));
    assert_eq!(legacy_tracer_name(&env), expected);
}

#[test]
fn cached_dynamic_tracer_builds_once_per_key() {
    let (sender, receiver) = sync_channel(4);
    let cache = cache(config(Exporter::Console, None, None), 4);
    let key = config_key("console");
    let built = Cell::new(0_usize);
    cache
        .cached_dynamic_tracer(key.clone(), true, || {
            built.set(built.get() + 1);
            Ok(provider("console", &sender))
        })
        .expect("first tracer");
    cache
        .cached_dynamic_tracer(key.clone(), true, || {
            built.set(built.get() + 1);
            Ok(provider("console", &sender))
        })
        .expect("cached tracer");
    assert_eq!(built.get(), 1);
    assert_eq!(cache.cached_keys(), vec![key]);
    assert_eq!(cache.join_shutdowns(), 0);
    assert!(drain(&receiver).is_empty());
}

#[test]
fn a_hit_moves_its_entry_to_the_most_recent_place() {
    let (sender, receiver) = sync_channel(8);
    let cache = cache(config(Exporter::Console, None, None), 2);
    let first = config_key("first");
    let second = config_key("second");
    let third = config_key("third");
    cache
        .cached_dynamic_tracer(first.clone(), true, || Ok(provider("first", &sender)))
        .expect("first tracer");
    cache
        .cached_dynamic_tracer(second, true, || Ok(provider("second", &sender)))
        .expect("second tracer");
    cache
        .cached_dynamic_tracer(first.clone(), true, || Ok(provider("first", &sender)))
        .expect("reused tracer");
    cache
        .cached_dynamic_tracer(third.clone(), true, || Ok(provider("third", &sender)))
        .expect("third tracer");
    assert_eq!(cache.cached_keys(), vec![first, third]);
    assert_eq!(cache.join_shutdowns(), 1);
    assert_eq!(shut_down_on_own_thread(&drain(&receiver)), vec!["second"]);
}

#[test]
fn a_provider_that_owns_its_exporter_is_shut_down_off_the_callers_thread() {
    let (sender, receiver) = sync_channel(8);
    let cache = cache(config(Exporter::Console, None, None), 1);
    cache
        .cached_dynamic_tracer(config_key("first"), true, || Ok(provider("first", &sender)))
        .expect("first tracer");
    cache
        .cached_dynamic_tracer(config_key("second"), true, || {
            Ok(provider("second", &sender))
        })
        .expect("second tracer");
    assert_eq!(cache.join_shutdowns(), 1);
    let records = drain(&receiver);
    assert_eq!(shut_down_on_own_thread(&records), vec!["first"]);
    assert_eq!(cache.cached_keys(), vec![config_key("second")]);
}

#[test]
fn a_provider_that_does_not_own_its_exporter_is_never_shut_down_by_the_cache() {
    let (sender, receiver) = sync_channel(8);
    let cache = cache(config(Exporter::Console, None, None), 1);
    cache
        .cached_dynamic_tracer(config_key("shared"), false, || {
            Ok(provider("shared", &sender))
        })
        .expect("shared tracer");
    cache
        .cached_dynamic_tracer(config_key("other"), true, || Ok(provider("other", &sender)))
        .expect("other tracer");
    assert_eq!(cache.join_shutdowns(), 0);
    let records = drain(&receiver);
    assert!(
        shut_down_on_own_thread(&records).is_empty(),
        "the shared exporter must stay alive"
    );
}

#[test]
fn the_shutdown_rule_follows_the_dropped_provider_not_the_request() {
    let (sender, receiver) = sync_channel(8);
    let cache = cache(config(Exporter::Console, None, None), 1);
    cache
        .cached_dynamic_tracer(config_key("owner"), true, || Ok(provider("owner", &sender)))
        .expect("owner tracer");
    cache
        .cached_dynamic_tracer(config_key("shared"), false, || {
            Ok(provider("shared", &sender))
        })
        .expect("shared tracer");
    assert_eq!(cache.join_shutdowns(), 1);
    assert_eq!(
        shut_down_on_own_thread(&drain(&receiver)),
        vec!["owner"],
        "the request that evicts a provider says nothing about whether it may be shut down"
    );
}

#[test]
fn above_the_maximum_the_least_recently_used_provider_is_dropped() {
    let (sender, receiver) = sync_channel(8);
    let cache = cache(config(Exporter::Console, None, None), 2);
    cache
        .cached_dynamic_tracer(config_key("first"), true, || Ok(provider("first", &sender)))
        .expect("first tracer");
    cache
        .cached_dynamic_tracer(config_key("second"), true, || {
            Ok(provider("second", &sender))
        })
        .expect("second tracer");
    cache
        .cached_dynamic_tracer(config_key("third"), true, || Ok(provider("third", &sender)))
        .expect("third tracer");
    assert_eq!(
        cache.cached_keys(),
        vec![config_key("second"), config_key("third")]
    );
    assert_eq!(cache.join_shutdowns(), 1);
    assert_eq!(shut_down_on_own_thread(&drain(&receiver)), vec!["first"]);
}

#[test]
fn a_maximum_below_one_is_raised_to_one() {
    let (sender, receiver) = sync_channel(8);
    let cache = cache(config(Exporter::Console, None, None), 0);
    cache
        .cached_dynamic_tracer(config_key("first"), true, || Ok(provider("first", &sender)))
        .expect("first tracer");
    cache
        .cached_dynamic_tracer(config_key("second"), true, || {
            Ok(provider("second", &sender))
        })
        .expect("second tracer");
    assert_eq!(cache.cached_keys(), vec![config_key("second")]);
    assert_eq!(cache.join_shutdowns(), 1);
    assert_eq!(shut_down_on_own_thread(&drain(&receiver)), vec!["first"]);
}

#[test]
fn a_build_error_is_returned_and_nothing_is_cached() {
    let cache = cache(config(Exporter::Console, None, None), 4);
    let attempts = Cell::new(0_usize);
    let outcome = cache.cached_dynamic_tracer(config_key("console"), true, || {
        attempts.set(attempts.get() + 1);
        Err(build_error())
    });
    outcome.unwrap_err();
    assert!(cache.cached_keys().is_empty());
    let retried = cache.cached_dynamic_tracer(config_key("console"), true, || {
        attempts.set(attempts.get() + 1);
        Err(build_error())
    });
    retried.unwrap_err();
    assert_eq!(attempts.get(), 2);
}

#[test]
fn a_provider_built_while_another_caller_cached_the_key_is_dropped() {
    let (sender, receiver) = sync_channel(8);
    let cache = cache(config(Exporter::Console, None, None), 4);
    let key = config_key("console");
    cache
        .cached_dynamic_tracer(key.clone(), true, || {
            let inner_key = key.clone();
            let inner_sender = sender.clone();
            cache
                .cached_dynamic_tracer(inner_key, true, || Ok(provider("inner", &inner_sender)))
                .expect("the racing tracer");
            Ok(provider("outer", &sender))
        })
        .expect("tracer");
    assert_eq!(cache.cached_keys(), vec![key]);
    assert_eq!(cache.join_shutdowns(), 1);
    assert_eq!(shut_down_on_own_thread(&drain(&receiver)), vec!["outer"]);
}

#[test]
fn tracer_for_config_keys_on_the_exporter_text_endpoint_and_headers() {
    let cache = cache(config(Exporter::Console, None, None), 8);
    let first = config(
        Exporter::Named("otlp_http".to_owned()),
        Some("http://a:4318"),
        None,
    );
    let same = config(
        Exporter::Named("otlp_http".to_owned()),
        Some("http://a:4318"),
        None,
    );
    let other = config(
        Exporter::Named("otlp_http".to_owned()),
        Some("http://b:4318"),
        None,
    );
    cache.tracer_for_config(&first).expect("first tracer");
    cache.tracer_for_config(&same).expect("reused tracer");
    cache.tracer_for_config(&other).expect("second tracer");
    assert_eq!(
        cache.cached_keys(),
        vec![
            CacheKey::Config {
                exporter: "otlp_http".to_owned(),
                endpoint: Some("http://a:4318".to_owned()),
                headers: None,
            },
            CacheKey::Config {
                exporter: "otlp_http".to_owned(),
                endpoint: Some("http://b:4318".to_owned()),
                headers: None,
            },
        ]
    );
}

#[test]
fn tracer_for_config_reports_a_headers_error_without_caching() {
    let cache = cache(config(Exporter::Console, None, None), 8);
    let broken = config(Exporter::Console, None, Some("oops"));
    cache.tracer_for_config(&broken).unwrap_err();
    assert!(cache.cached_keys().is_empty());
}

#[test]
fn tracer_for_headers_keys_on_the_whole_map() {
    let cache = cache(
        config(
            Exporter::Named("otlp_http".to_owned()),
            Some("http://a:4318"),
            None,
        ),
        8,
    );
    let first = map(&[("authorization", "Bearer one")]);
    let same = map(&[("authorization", "Bearer one")]);
    let other = map(&[("authorization", "Bearer two")]);
    cache.tracer_for_headers(&first).expect("first tracer");
    cache.tracer_for_headers(&same).expect("reused tracer");
    cache.tracer_for_headers(&other).expect("second tracer");
    assert_eq!(
        cache.cached_keys(),
        vec![CacheKey::Headers(first), CacheKey::Headers(other)]
    );
}

#[test]
fn tracer_for_headers_with_an_empty_map_uses_the_configured_headers() {
    let cache = cache(config(Exporter::InMemory, None, Some("x-config=1")), 8);
    cache
        .tracer_for_headers(&BTreeMap::new())
        .expect("a tracer from the configured headers");
    assert_eq!(
        cache.cached_keys(),
        vec![CacheKey::Headers(BTreeMap::new())]
    );
}

#[test]
fn a_dynamic_header_reaches_the_exporters_post() {
    let recorder = Arc::new(Recorder::default());
    let cache = cache_with(
        config(
            Exporter::Named("otlp_http".to_owned()),
            Some("http://collector:4318"),
            None,
        ),
        1,
        InMemorySpanExporter::default(),
        recording_clients(&recorder),
    );
    let tracer = cache
        .tracer_for_headers(&map(&[("authorization", "Bearer-one")]))
        .expect("first tracer");
    tracer.start("operation").end();
    drop(tracer);
    cache
        .tracer_for_headers(&map(&[("authorization", "Bearer-two")]))
        .expect("the evicting tracer");
    assert_eq!(cache.join_shutdowns(), 1);
    let posted = recorder.take();
    assert_eq!(posted[0].uri, "http://collector:4318/v1/traces");
    assert_eq!(header(&posted[0], "authorization"), Some("Bearer-one"));
}

#[test]
fn an_empty_dynamic_map_posts_the_configured_headers() {
    let recorder = Arc::new(Recorder::default());
    let cache = cache_with(
        config(
            Exporter::Named("otlp_http".to_owned()),
            Some("http://collector:4318"),
            Some("x-config=1"),
        ),
        1,
        InMemorySpanExporter::default(),
        recording_clients(&recorder),
    );
    let tracer = cache
        .tracer_for_headers(&BTreeMap::new())
        .expect("a tracer from the configured headers");
    tracer.start("operation").end();
    drop(tracer);
    cache
        .tracer_for_headers(&map(&[("x-other", "2")]))
        .expect("the evicting tracer");
    assert_eq!(cache.join_shutdowns(), 1);
    assert_eq!(header(&recorder.take()[0], "x-config"), Some("1"));
}

#[test]
fn tracer_for_config_posts_to_the_overrides_normalized_endpoint() {
    let recorder = Arc::new(Recorder::default());
    let cache = cache_with(
        config(
            Exporter::Named("otlp_http".to_owned()),
            Some("http://logger:4318"),
            None,
        ),
        1,
        InMemorySpanExporter::default(),
        recording_clients(&recorder),
    );
    let override_config = config(
        Exporter::Named("otlp_http".to_owned()),
        Some("http://override:4318/v1/logs"),
        None,
    );
    let tracer = cache
        .tracer_for_config(&override_config)
        .expect("the override tracer");
    tracer.start("operation").end();
    drop(tracer);
    cache
        .tracer_for_config(&config(
            Exporter::Named("otlp_http".to_owned()),
            Some("http://other:4318"),
            None,
        ))
        .expect("the evicting tracer");
    assert_eq!(cache.join_shutdowns(), 1);
    let posted = recorder.take();
    assert_eq!(posted[0].uri, "http://override:4318/v1/traces");
}

#[test]
fn an_evicted_provider_never_shuts_the_shared_exporter_down() {
    let shared = InMemorySpanExporter::default();
    let cache = cache_with(
        config(Exporter::Console, None, None),
        1,
        shared.clone(),
        clients(),
    );
    let tracer = cache
        .tracer_for_config(&config(Exporter::InMemory, None, None))
        .expect("a tracer over the shared exporter");
    tracer.start("shared-span").end();
    cache
        .tracer_for_config(&config(Exporter::Console, None, None))
        .expect("the evicting tracer");
    assert_eq!(cache.join_shutdowns(), 0);
    let spans = shared.get_finished_spans().expect("spans");
    assert_eq!(spans.len(), 1);
    assert!(!shared.is_shutdown_called());
}

#[test]
fn an_evicted_headers_provider_never_shuts_the_shared_exporter_down() {
    let shared = InMemorySpanExporter::default();
    let cache = cache_with(
        config(Exporter::InMemory, None, None),
        1,
        shared.clone(),
        clients(),
    );
    let tracer = cache
        .tracer_for_headers(&map(&[("authorization", "one")]))
        .expect("a tracer over the shared exporter");
    tracer.start("shared-span").end();
    cache
        .tracer_for_headers(&map(&[("authorization", "two")]))
        .expect("the evicting tracer");
    assert_eq!(cache.join_shutdowns(), 0);
    let spans = shared.get_finished_spans().expect("spans");
    assert_eq!(spans.len(), 1);
    assert!(!shared.is_shutdown_called());
}

#[test]
fn a_cache_key_debug_never_prints_header_values() {
    let key = CacheKey::Headers(map(&[("authorization", "Bearer secret")]));
    let text = format!("{key:?}");
    assert!(text.contains("authorization"));
    assert!(!text.contains("Bearer secret"));
}

#[test]
fn a_config_cache_key_debug_never_prints_header_values() {
    let key = CacheKey::Config {
        exporter: "otlp_http".to_owned(),
        endpoint: Some("http://collector:4318".to_owned()),
        headers: Some("authorization=Bearer secret".to_owned()),
    };
    let text = format!("{key:?}");
    assert!(text.contains("otlp_http"));
    assert!(!text.contains("Bearer secret"));
}

#[test]
fn a_cached_tracer_keeps_the_configured_instrumentation_scope() {
    let shared = InMemorySpanExporter::default();
    let cache = DynamicTracerCache::new(
        CacheSettings {
            config: config(Exporter::InMemory, None, None),
            callback_name: None,
            resource: Resource::builder_empty().build(),
            tracer_name: "proxy-tracer".to_owned(),
            max_providers: 4,
        },
        LegacyExporterBuilder::new(clients(), shared.clone()),
    );
    let tracer = cache
        .tracer_for_config(&config(Exporter::InMemory, None, None))
        .expect("a tracer over the shared exporter");
    tracer.start("operation").end();
    let spans = shared.get_finished_spans().expect("spans");
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].instrumentation_scope.name(), "proxy-tracer");
}

#[test]
fn a_cached_tracer_carries_the_cache_resource() {
    let recorder = Arc::new(Recorder::default());
    let resource = Resource::builder_empty()
        .with_attribute(KeyValue::new("service.name", "checkout"))
        .build();
    let cache = DynamicTracerCache::new(
        CacheSettings {
            config: otlp("http://collector:4318"),
            callback_name: None,
            resource,
            tracer_name: "litellm".to_owned(),
            max_providers: 1,
        },
        LegacyExporterBuilder::new(
            recording_clients(&recorder),
            InMemorySpanExporter::default(),
        ),
    );
    let tracer = cache
        .tracer_for_config(&otlp("http://collector:4318"))
        .expect("the exporting tracer");
    tracer.start("operation").end();
    drop(tracer);
    cache
        .tracer_for_config(&otlp("http://other:4318"))
        .expect("the evicting tracer");
    assert_eq!(cache.join_shutdowns(), 1);
    let posted = recorder.take();
    assert!(
        posted[0]
            .body
            .windows(8)
            .any(|window| window == b"checkout"),
        "the exported span must carry the cache resource"
    );
}

#[test]
fn tracer_for_config_accepts_the_in_memory_exporter() {
    let cache = cache(
        config(Exporter::Named("otlp_http".to_owned()), None, None),
        8,
    );
    cache
        .tracer_for_config(&config(Exporter::InMemory, None, None))
        .expect("a tracer over the shared exporter");
    assert_eq!(
        cache.cached_keys(),
        vec![CacheKey::Config {
            exporter: "in_memory".to_owned(),
            endpoint: None,
            headers: None,
        }]
    );
}
