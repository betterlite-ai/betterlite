use std::collections::BTreeMap;
use std::mem::take;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_models::api_base_lookup::ApiBaseError;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::alert_dispatch::{Alert, AlertLevel};
use crate::alert_settings::records::HangingRequestData;
use crate::hanging_requests::cache::{HangingCacheFactory, HangingRequestCache};
use crate::hanging_requests::check::{AlertingHangingRequestCheck, HangingCheckParts};
use crate::hanging_requests::clock::Clock;
use crate::hanging_requests::status::CompletionStatusCache;
use crate::latency_alerts::api_base::ApiBaseResolver;
use crate::latency_alerts::settings::{RequestAlertSettings, RequestAlertSettingsSource};
use crate::latency_alerts::sink::AlertSink;

pub(crate) const NOW: f64 = 1_700_000_000.0;

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the test JSON parses")
}

pub(crate) fn object(pairs: &[(&str, Json)]) -> IndexMap<String, Json> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn metadata(text: &str) -> IndexMap<String, Json> {
    let Json::Object(map) = json(text) else {
        panic!("the test metadata is a JSON object");
    };
    map
}

pub(crate) fn alert_settings(threshold_seconds: f64) -> RequestAlertSettings {
    RequestAlertSettings {
        alerting_configured: true,
        alert_types: vec![AlertType::LlmTooSlow, AlertType::LlmRequestsHanging],
        alerting_threshold_seconds: threshold_seconds,
    }
}

pub(crate) fn hanging_record(request_id: &str, model: &str, created_at: f64) -> HangingRequestData {
    HangingRequestData::new(request_id.to_owned(), model.to_owned(), created_at)
}

pub(crate) struct RecordedAlert {
    pub(crate) message: String,
    pub(crate) level: AlertLevel,
    pub(crate) alert_type: AlertType,
    pub(crate) alerting_metadata: IndexMap<String, Json>,
    pub(crate) request_model: Option<String>,
    pub(crate) api_base: Option<String>,
    pub(crate) kwargs: IndexMap<String, Json>,
}

#[derive(Default)]
pub(crate) struct RecordingSink {
    alerts: Mutex<Vec<RecordedAlert>>,
}

impl RecordingSink {
    pub(crate) fn take(&self) -> Vec<RecordedAlert> {
        let mut alerts = self.alerts.lock().unwrap();
        take(&mut *alerts)
    }
}

#[async_trait]
impl AlertSink for RecordingSink {
    async fn send_alert(&self, alert: Alert<'_>) {
        let recorded = RecordedAlert {
            message: alert.message.to_owned(),
            level: alert.level,
            alert_type: alert.alert_type,
            alerting_metadata: alert.alerting_metadata.clone(),
            request_model: alert.request_model.map(str::to_owned),
            api_base: alert.api_base.map(str::to_owned),
            kwargs: alert.kwargs.clone(),
        };
        self.alerts.lock().unwrap().push(recorded);
    }
}

pub(crate) struct FixedResolver {
    answer: Result<Option<String>, ApiBaseError>,
    calls: Mutex<Vec<(String, IndexMap<String, Json>)>>,
}

impl FixedResolver {
    pub(crate) fn new(api_base: Option<&str>) -> Self {
        Self {
            answer: Ok(api_base.map(str::to_owned)),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn failing() -> Self {
        Self {
            answer: Err(ApiBaseError::InvalidVertexLocation {
                message: "bad location".to_owned(),
            }),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<(String, IndexMap<String, Json>)> {
        self.calls.lock().unwrap().clone()
    }
}

impl ApiBaseResolver for FixedResolver {
    fn resolve(
        &self,
        model: &str,
        params: &IndexMap<String, Json>,
    ) -> Result<Option<String>, ApiBaseError> {
        self.calls
            .lock()
            .unwrap()
            .push((model.to_owned(), params.clone()));
        self.answer.clone()
    }
}

pub(crate) struct FixedSettings {
    settings: Mutex<RequestAlertSettings>,
}

impl FixedSettings {
    pub(crate) const fn new(settings: RequestAlertSettings) -> Self {
        Self {
            settings: Mutex::new(settings),
        }
    }

    pub(crate) fn set(&self, settings: RequestAlertSettings) {
        *self.settings.lock().unwrap() = settings;
    }

    pub(crate) fn set_threshold(&self, seconds: f64) {
        self.settings.lock().unwrap().alerting_threshold_seconds = seconds;
    }
}

impl RequestAlertSettingsSource for FixedSettings {
    fn current(&self) -> RequestAlertSettings {
        self.settings.lock().unwrap().clone()
    }
}

struct Stored {
    data: HangingRequestData,
    ttl: Duration,
}

#[derive(Default)]
pub(crate) struct FakeHangingCache {
    entries: Mutex<IndexMap<String, Stored>>,
    vanished: Mutex<Vec<String>>,
    oldest_calls: AtomicUsize,
}

impl FakeHangingCache {
    pub(crate) fn vanish(&self, key: &str) {
        self.vanished.lock().unwrap().push(key.to_owned());
    }

    pub(crate) fn seed(&self, data: HangingRequestData) {
        let key = data.request_id.clone();
        self.entries.lock().unwrap().insert(
            key,
            Stored {
                data,
                ttl: Duration::from_secs(1),
            },
        );
    }

    pub(crate) fn stored(&self, key: &str) -> Option<HangingRequestData> {
        self.entries
            .lock()
            .unwrap()
            .get(key)
            .map(|stored| stored.data.clone())
    }

    pub(crate) fn stored_ttl(&self, key: &str) -> Option<Duration> {
        self.entries
            .lock()
            .unwrap()
            .get(key)
            .map(|stored| stored.ttl)
    }

    pub(crate) fn entry_count(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    pub(crate) fn oldest_calls(&self) -> usize {
        self.oldest_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl HangingRequestCache for FakeHangingCache {
    async fn insert(&self, key: &str, value: &HangingRequestData, ttl: Duration) {
        self.entries.lock().unwrap().insert(
            key.to_owned(),
            Stored {
                data: value.clone(),
                ttl,
            },
        );
    }

    async fn get(&self, key: &str) -> Option<HangingRequestData> {
        self.entries
            .lock()
            .unwrap()
            .get(key)
            .map(|stored| stored.data.clone())
    }

    async fn oldest_keys(&self, limit: usize) -> Vec<String> {
        let _previous = self.oldest_calls.fetch_add(1, Ordering::SeqCst);
        let mut keys: Vec<String> = self
            .entries
            .lock()
            .unwrap()
            .keys()
            .take(limit)
            .cloned()
            .collect();
        keys.extend(self.vanished.lock().unwrap().iter().cloned());
        keys
    }

    fn remove(&self, key: &str) {
        let mut entries = self.entries.lock().unwrap();
        let _removed = entries.shift_remove(key);
    }

    async fn replace(&self, key: &str, value: &HangingRequestData) {
        let mut entries = self.entries.lock().unwrap();
        if let Some(stored) = entries.get_mut(key) {
            stored.data.clone_from(value);
        }
    }
}

pub(crate) struct RecordingFactory {
    cache: Arc<FakeHangingCache>,
    ttls: Mutex<Vec<Duration>>,
}

impl RecordingFactory {
    pub(crate) const fn new(cache: Arc<FakeHangingCache>) -> Self {
        Self {
            cache,
            ttls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn ttls(&self) -> Vec<Duration> {
        self.ttls.lock().unwrap().clone()
    }
}

impl HangingCacheFactory for RecordingFactory {
    fn create(&self, ttl: Duration) -> Arc<dyn HangingRequestCache> {
        self.ttls.lock().unwrap().push(ttl);
        Arc::clone(&self.cache)
    }
}

#[derive(Default)]
pub(crate) struct FakeStatus {
    values: Mutex<BTreeMap<String, Json>>,
    reads: Mutex<Vec<String>>,
}

impl FakeStatus {
    pub(crate) fn set(&self, key: &str, value: Json) {
        self.values.lock().unwrap().insert(key.to_owned(), value);
    }

    pub(crate) fn reads(&self) -> Vec<String> {
        self.reads.lock().unwrap().clone()
    }
}

#[async_trait]
impl CompletionStatusCache for FakeStatus {
    async fn local_get(&self, key: &str) -> Option<Json> {
        self.reads.lock().unwrap().push(key.to_owned());
        self.values.lock().unwrap().get(key).cloned()
    }
}

pub(crate) fn clock_at(now: &Arc<Mutex<f64>>) -> Clock {
    let handle = Arc::clone(now);
    Arc::new(move || *handle.lock().unwrap())
}

pub(crate) struct Fixture {
    pub(crate) check: Arc<AlertingHangingRequestCheck>,
    pub(crate) sink: Arc<RecordingSink>,
    pub(crate) resolver: Arc<FixedResolver>,
    pub(crate) settings: Arc<FixedSettings>,
    pub(crate) cache: Arc<FakeHangingCache>,
    pub(crate) factory: Arc<RecordingFactory>,
    pub(crate) status: Arc<FakeStatus>,
    now: Arc<Mutex<f64>>,
}

impl Fixture {
    pub(crate) fn set_now(&self, seconds: f64) {
        *self.now.lock().unwrap() = seconds;
    }
}

pub(crate) fn fixture() -> Fixture {
    fixture_with(
        alert_settings(300.0),
        FixedResolver::new(Some("https://api.openai.com")),
    )
}

pub(crate) fn fixture_with(settings: RequestAlertSettings, resolver: FixedResolver) -> Fixture {
    let sink = Arc::new(RecordingSink::default());
    let resolver = Arc::new(resolver);
    let fixed_settings = Arc::new(FixedSettings::new(settings));
    let status = Arc::new(FakeStatus::default());
    let cache = Arc::new(FakeHangingCache::default());
    let factory = Arc::new(RecordingFactory::new(Arc::clone(&cache)));
    let now = Arc::new(Mutex::new(NOW));
    let check = Arc::new(AlertingHangingRequestCheck::new(HangingCheckParts {
        sink: Arc::clone(&sink),
        api_base: Arc::clone(&resolver),
        settings: Arc::clone(&fixed_settings),
        completed: Arc::clone(&status),
        caches: Arc::clone(&factory),
        clock: clock_at(&now),
    }));
    Fixture {
        check,
        sink,
        resolver,
        settings: fixed_settings,
        cache,
        factory,
        status,
        now,
    }
}
