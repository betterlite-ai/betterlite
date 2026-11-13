use std::collections::BTreeMap;
use std::future::pending;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::sections::huggingface_hub::{
    OfflineSetting, Settings as HubSettings, TransferSetting,
};
use betterlite_config::sections::huggingface_oidc::Settings as OidcSettings;
use betterlite_config::sections::routing_state_store::connection::Connection;
use betterlite_config::types::ProxyTrust;
use betterlite_http::hub_oauth::clock::{HubClock, SystemHubClock};
use betterlite_http::hub_token::{HubTokenFileReader, HubTokenResolver, TokioHubTokenFileReader};
use betterlite_http::proxy::ProxiedClient;
use betterlite_models::catalog::{BundledCatalog, ModelCatalog};
use betterlite_models::decisions::Json;
use betterlite_tokenizer::count::Counter;
use betterlite_tokenizer::count::settings::Settings as TokenizerSettings;
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::fetch::TiktokenFetcher;
use betterlite_tokenizer::engine::hub::HubDownloader;
use betterlite_tokenizer::engine::limits::{
    BlockingCounter, BuildJob, ConvertJob, CountJob, CountingService, EncodeLimits,
};
use betterlite_tokenizer::engine::selection::Selector;
use betterlite_tokenizer::reservation::RequestBody;
use betterlite_tokenizer::reservation::schedule::BoundedPool;
use indexmap::IndexMap;
use serde_json::Number;
use tokio::time::Instant;

use crate::io_token_reservations::clock::{IoClock, minute_label};
use crate::io_token_reservations::counter::{CountObservation, IoCounter};
use crate::io_token_reservations::estimate::{
    InputTokenCounter, IoInputField, TokenizerInputCounter,
};
use crate::io_token_reservations::limits::{IoDeployment, IoLimits, MinuteKeys, minute_keys};
use crate::io_token_reservations::service::IoTokenReservations;
use crate::redis_failure_policy::Settings as BreakerSettings;
use crate::routing_state_store::test_support::FakeRedis;
use crate::routing_state_store::{BreakerClock, RedisSettings, RoutingStateStore, Settings};
use crate::routing_value_store::{RoutingValueStore, StoreSettings};

pub(crate) const NOW_SECONDS: u64 = 1_700_000_000;

pub(crate) fn run_case_table(name: &str, cases: &[(&'static str, fn())]) {
    for (case_name, run) in cases {
        if *case_name == name {
            run();
            return;
        }
    }
    panic!("unknown case {name}");
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Behavior {
    Record,
    Unavailable,
    Pending,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CounterCall {
    pub(crate) key: String,
    pub(crate) value: i64,
    pub(crate) ttl_seconds: u64,
}

pub(crate) struct RecordingCounter {
    values: Mutex<BTreeMap<String, i64>>,
    calls: Mutex<Vec<CounterCall>>,
    itpm: Behavior,
    otpm: Behavior,
    fail_releases: AtomicBool,
}

impl RecordingCounter {
    pub(crate) fn new(itpm: Behavior, otpm: Behavior) -> Self {
        Self {
            values: Mutex::new(BTreeMap::new()),
            calls: Mutex::new(Vec::new()),
            itpm,
            otpm,
            fail_releases: AtomicBool::new(false),
        }
    }

    pub(crate) fn with_failing_releases(self) -> Self {
        self.fail_releases.store(true, Ordering::SeqCst);
        self
    }

    pub(crate) fn seed(&self, key: &str, value: i64) {
        if let Ok(mut values) = self.values.lock() {
            values.insert(key.to_owned(), value);
        }
    }

    pub(crate) fn value(&self, key: &str) -> i64 {
        self.values
            .lock()
            .ok()
            .and_then(|values| values.get(key).copied())
            .unwrap_or_default()
    }

    pub(crate) fn calls(&self) -> Vec<CounterCall> {
        self.calls
            .lock()
            .map(|calls| calls.clone())
            .unwrap_or_default()
    }

    fn behavior(&self, key: &str) -> Behavior {
        if key.contains(":itpm:") {
            self.itpm
        } else {
            self.otpm
        }
    }
}

#[async_trait]
impl IoCounter for RecordingCounter {
    async fn increment_counter(&self, key: &str, value: i64, ttl_seconds: u64) -> CountObservation {
        if value < 0 && self.fail_releases.load(Ordering::SeqCst) {
            return CountObservation::Unavailable;
        }
        match self.behavior(key) {
            Behavior::Unavailable => return CountObservation::Unavailable,
            Behavior::Pending => pending::<()>().await,
            Behavior::Record => {}
        }
        let Ok(mut calls) = self.calls.lock() else {
            return CountObservation::Unavailable;
        };
        calls.push(CounterCall {
            key: key.to_owned(),
            value,
            ttl_seconds,
        });
        drop(calls);
        let Ok(mut values) = self.values.lock() else {
            return CountObservation::Unavailable;
        };
        let Some(current) = values
            .get(key)
            .copied()
            .unwrap_or_default()
            .checked_add(value)
        else {
            return CountObservation::Overflow;
        };
        values.insert(key.to_owned(), current);
        drop(values);
        CountObservation::Counted(current)
    }
}

pub(crate) struct FixedEstimator {
    pub(crate) estimate: Option<i64>,
}

#[async_trait]
impl InputTokenCounter for FixedEstimator {
    async fn count_field(
        &self,
        _model: &str,
        _field: IoInputField,
        _value: &Json,
    ) -> Option<usize> {
        self.estimate
            .and_then(|estimate| usize::try_from(estimate).ok())
    }
}

pub(crate) fn now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(NOW_SECONDS))
        .unwrap_or(UNIX_EPOCH)
}

pub(crate) fn minute() -> String {
    minute_label(now())
}

pub(crate) fn clock() -> IoClock {
    Arc::new(now)
}

pub(crate) fn catalog() -> Arc<ModelCatalog> {
    BundledCatalog::default().get()
}

pub(crate) fn service<T>(counter: Arc<T>, estimate: Option<i64>) -> IoTokenReservations
where
    T: IoCounter + 'static,
{
    IoTokenReservations {
        counter,
        estimator: Arc::new(FixedEstimator { estimate }),
        catalog: catalog(),
        clock: clock(),
    }
}

pub(crate) fn deployment(itpm: Option<i64>, otpm: Option<i64>) -> IoDeployment<'static> {
    IoDeployment {
        model_id: Some("dep-a"),
        model: Some("gpt-4o"),
        limits: IoLimits { itpm, otpm },
    }
}

pub(crate) fn keys(deployment: &IoDeployment<'_>) -> MinuteKeys {
    minute_keys(deployment, &minute()).unwrap()
}

pub(crate) fn number(value: i64) -> Json {
    Json::Number(Number::from(value))
}

pub(crate) fn object(fields: &[(&str, Json)]) -> Json {
    let mut map = IndexMap::new();
    for (key, value) in fields {
        map.insert((*key).to_owned(), value.clone());
    }
    Json::Object(map)
}

pub(crate) fn local_store() -> RoutingValueStore {
    RoutingValueStore::new(
        StoreSettings {
            max_size_in_memory: None,
            default_ttl_seconds: None,
            max_size_per_item: None,
            loaded_max_size_per_item_kib: 1024,
        },
        Arc::new(|| 0.0_f64),
    )
    .unwrap()
}

pub(crate) fn messages() -> Json {
    let mut message = IndexMap::new();
    message.insert("role".to_owned(), Json::Text("user".to_owned()));
    message.insert("content".to_owned(), Json::Text("hello".to_owned()));
    Json::List(vec![Json::Object(message)])
}

pub(crate) fn body() -> RequestBody {
    let mut request = RequestBody::new();
    request.insert("messages".to_owned(), messages());
    request
}

pub(crate) fn body_with(field: &str, value: Json) -> RequestBody {
    let mut request = RequestBody::new();
    request.insert(field.to_owned(), value);
    request
}

struct InlineCounter;

#[async_trait]
impl BlockingCounter for InlineCounter {
    async fn count(&self, job: CountJob) -> Result<usize, EngineError> {
        job()
    }

    async fn build(&self, job: BuildJob) -> Result<Engine, EngineError> {
        job()
    }

    async fn convert(&self, job: ConvertJob) -> Result<Vec<u8>, EngineError> {
        job()
    }
}

fn hub_settings() -> HubSettings {
    HubSettings {
        endpoint: "http://127.0.0.1:1".to_owned(),
        offline: OfflineSetting::Online,
        etag_timeout: 10,
        download_timeout: 10,
        disable_implicit_token: false,
        env_token: None,
        hf_home: PathBuf::from("unused"),
        hub_cache: PathBuf::from("unused"),
        token_path: PathBuf::from("unused-token"),
        stored_tokens_path: PathBuf::from("unused-stored-tokens"),
        transfer: TransferSetting::Standard,
    }
}

fn hub_downloader(client: &Arc<ProxiedClient>) -> Arc<HubDownloader> {
    let hub = hub_settings();
    let reader: Arc<dyn HubTokenFileReader> = Arc::new(TokioHubTokenFileReader);
    let clock: Arc<dyn HubClock> = Arc::new(SystemHubClock::default());
    let resolver = Arc::new(HubTokenResolver::new(
        hub.clone(),
        OidcSettings::default(),
        Arc::clone(client),
        reader,
        clock,
    ));
    Arc::new(HubDownloader::new(
        Arc::new(hub),
        resolver,
        Arc::new(Instant::now),
    ))
}

pub(crate) fn tokenizer_estimator(counter: Arc<Counter>) -> TokenizerInputCounter {
    TokenizerInputCounter {
        counter,
        pool: Arc::new(BoundedPool::new(4)),
    }
}

pub(crate) fn token_counter() -> Counter {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new()).unwrap();
    let client = Arc::new(ProxiedClient::outbound(&Arc::new(policy)).unwrap());
    let service = Arc::new(CountingService::new(
        4,
        Arc::new(InlineCounter),
        EncodeLimits::default(),
    ));
    let fetcher = Arc::new(TiktokenFetcher::new(
        Arc::clone(&client),
        None,
        Arc::clone(&service),
    ));
    let catalog = BundledCatalog::default().get();
    let selector = Arc::new(Selector::new(
        Arc::clone(&fetcher),
        hub_downloader(&client),
        Arc::clone(&catalog),
        Arc::clone(&service),
        false,
    ));
    Counter::new(
        selector,
        fetcher,
        service,
        catalog,
        TokenizerSettings::default(),
    )
}

pub(crate) struct StoreFixture {
    pub(crate) store: RoutingStateStore,
    pub(crate) server: FakeRedis,
}

pub(crate) async fn store_fixture() -> StoreFixture {
    let server = FakeRedis::start().await;
    let settings = Settings {
        redis: RedisSettings {
            url: None,
            host: Some("127.0.0.1".to_owned()),
            port: Some(i64::from(server.port())),
            password: None,
            namespace: None,
            connection: Connection::default(),
            socket_timeout: 2.0_f64,
            pool_timeout_seconds: 5,
            environment: BTreeMap::new(),
        },
        breaker: BreakerSettings {
            enabled: true,
            failure_threshold: 3,
            recovery_timeout: Duration::from_secs(60),
            timeout_min_duration: Duration::from_secs(5),
            timeout_log_interval: Duration::from_secs(1),
        },
    };
    let breaker_clock: BreakerClock = Arc::new(|| Duration::ZERO);
    let store = RoutingStateStore::new(&settings, breaker_clock).unwrap();
    StoreFixture { store, server }
}
