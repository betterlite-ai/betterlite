use std::collections::{BTreeMap, VecDeque};
use std::error::Error;
use std::future::{Future, ready};
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::sync::watch;
use tokio::task::yield_now;
use tokio::time::{sleep, timeout};
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::util::SubscriberInitExt as _;

use crate::config_sync::clock::ConfigSyncClock;
use crate::config_sync::endpoint::{
    ConfigSyncMessage, ConfigSyncPubSubClient, ConfigSyncSubscription,
};
use crate::config_sync::error::ConfigSyncError;
use crate::config_sync::jitter::JitterSource;
use crate::config_sync::publish::{PublishChange, PublishFuture};
use crate::config_sync::settings::ConfigSyncSettings;
use crate::config_sync::subscriber::{ConfigSyncSubscriber, ResyncCallback};
use crate::config_sync::table_actions::TableActions;
use crate::redis_cache::RedisCache;
use crate::redis_cache::connect::{RedisCacheOptions, connect};
use crate::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use crate::redis_client_reuse::breaker_gate::BreakerGate;
use crate::redis_client_reuse::error::ClientReuseError;
use crate::redis_client_reuse::service_event::ServiceEvent;
use crate::redis_client_reuse::service_hooks::ServiceHooks;
use crate::redis_client_reuse::store::RedisClientStore;
use crate::redis_client_reuse::transport::RedisCacheFamily;
use crate::redis_connection_settings::model::settings::RedisOverrides;
use crate::redis_live_client::LiveClientFactory;
use crate::redis_live_client::tests::fake_server::FakeServer;

pub(crate) type TestSubscriber = ConfigSyncSubscriber<ScriptedClient, FakeClock, &'static str>;

#[derive(Clone, Default)]
pub(crate) struct Journal(Arc<Mutex<Vec<String>>>);

impl Journal {
    pub(crate) fn record(&self, entry: &str) {
        let mut entries = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        entries.push(entry.to_owned());
    }

    pub(crate) fn entries(&self) -> Vec<String> {
        let entries = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        entries.clone()
    }

    pub(crate) fn first_index(&self, entry: &str) -> Option<usize> {
        self.entries().iter().position(|recorded| recorded == entry)
    }
}

pub(crate) enum MessageStep {
    Message(&'static str),
    Empty,
    Fail,
}

pub(crate) enum ClientStep {
    Fail,
    Session(VecDeque<MessageStep>),
}

#[derive(Clone)]
pub(crate) struct ScriptedClient {
    pub(crate) steps: Arc<Mutex<VecDeque<ClientStep>>>,
    pub(crate) journal: Journal,
}

impl ScriptedClient {
    pub(crate) fn scripted(steps: Vec<ClientStep>, journal: &Journal) -> Self {
        Self {
            steps: Arc::new(Mutex::new(VecDeque::from(steps))),
            journal: journal.clone(),
        }
    }
}

impl ConfigSyncPubSubClient for ScriptedClient {
    type Subscription = ScriptedSubscription;

    fn subscribe(
        &self,
    ) -> impl Future<Output = Result<Self::Subscription, ConfigSyncError>> + Send {
        let step = {
            let mut steps = self.steps.lock().unwrap_or_else(PoisonError::into_inner);
            steps.pop_front()
        };
        self.journal.record("subscribe");
        let outcome = match step {
            Some(ClientStep::Session(steps)) => Ok(ScriptedSubscription {
                steps,
                journal: self.journal.clone(),
                closed: false,
            }),
            Some(ClientStep::Fail) | None => Err(scripted_error()),
        };
        ready(outcome)
    }
}

pub(crate) struct ScriptedSubscription {
    pub(crate) steps: VecDeque<MessageStep>,
    pub(crate) journal: Journal,
    pub(crate) closed: bool,
}

impl ConfigSyncSubscription for ScriptedSubscription {
    async fn next_message(
        &mut self,
        timeout: Duration,
    ) -> Result<Option<ConfigSyncMessage>, ConfigSyncError> {
        self.journal
            .record(&format!("next:{:.3}", timeout.as_secs_f64()));
        match self.steps.pop_front() {
            Some(MessageStep::Message(payload)) => Ok(Some(ConfigSyncMessage {
                channel: "litellm_proxy.config_change".to_owned(),
                payload: payload.as_bytes().to_vec(),
            })),
            Some(MessageStep::Empty) => Ok(None),
            Some(MessageStep::Fail) => Err(scripted_error()),
            None => {
                yield_now().await;
                Ok(None)
            }
        }
    }

    fn close(&mut self) -> impl Future<Output = ()> + Send {
        self.journal.record("close");
        self.closed = true;
        ready(())
    }
}

pub(crate) fn scripted_error() -> ConfigSyncError {
    ConfigSyncError::Client(ClientReuseError::ClusterPubSub)
}

pub(crate) fn session(steps: Vec<MessageStep>) -> ClientStep {
    ClientStep::Session(VecDeque::from(steps))
}

pub(crate) async fn subscription(client: &ScriptedClient) -> ScriptedSubscription {
    client.subscribe().await.expect("a scripted subscription")
}

#[derive(Default)]
pub(crate) struct FakeClock {
    pub(crate) now: Mutex<Duration>,
    pub(crate) pauses: Mutex<Vec<Duration>>,
    journal: Option<Journal>,
}

impl FakeClock {
    pub(crate) fn recording(journal: &Journal) -> Self {
        Self {
            now: Mutex::new(Duration::ZERO),
            pauses: Mutex::new(Vec::new()),
            journal: Some(journal.clone()),
        }
    }

    pub(crate) fn advance(&self, duration: Duration) {
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        *now = now.saturating_add(duration);
    }

    pub(crate) fn recorded_pauses(&self) -> Vec<Duration> {
        let pauses = self.pauses.lock().unwrap_or_else(PoisonError::into_inner);
        pauses.clone()
    }
}

impl ConfigSyncClock for FakeClock {
    fn now(&self) -> Duration {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn pause(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.pauses
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(duration);
        if let Some(journal) = &self.journal {
            journal.record(&format!("pause:{:.3}", duration.as_secs_f64()));
        }
        self.advance(duration);
        ready(())
    }
}

#[derive(Clone)]
pub(crate) struct FixedJitter {
    pub(crate) amount_nanos: u64,
    limits: Arc<Mutex<Vec<u64>>>,
}

impl FixedJitter {
    pub(crate) fn fixed(amount_nanos: u64) -> Self {
        Self {
            amount_nanos,
            limits: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn recorded_limits(&self) -> Vec<u64> {
        let limits = self.limits.lock().unwrap_or_else(PoisonError::into_inner);
        limits.clone()
    }

    fn record_limit(&self, upper_exclusive_nanos: u64) {
        self.limits
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(upper_exclusive_nanos);
    }
}

impl JitterSource for FixedJitter {
    fn uniform_nanos(&self, upper_exclusive_nanos: u64) -> u64 {
        self.record_limit(upper_exclusive_nanos);
        self.amount_nanos
    }
}

pub(crate) fn test_subscriber<C>(
    client: ScriptedClient,
    clock: C,
    jitter: FixedJitter,
    callbacks: Vec<ResyncCallback<&'static str>>,
) -> ConfigSyncSubscriber<ScriptedClient, C, &'static str>
where
    C: ConfigSyncClock,
{
    ConfigSyncSubscriber {
        client,
        clock,
        jitter: Arc::new(jitter),
        callbacks,
        settings: ConfigSyncSettings::default(),
    }
}

#[derive(Clone)]
pub(crate) struct SlowClock {
    journal: Journal,
}

impl SlowClock {
    pub(crate) fn recording(journal: &Journal) -> Self {
        Self {
            journal: journal.clone(),
        }
    }
}

impl ConfigSyncClock for SlowClock {
    fn now(&self) -> Duration {
        Duration::ZERO
    }

    fn pause(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        slow_pause(self.journal.clone(), duration)
    }
}

async fn slow_pause(journal: Journal, duration: Duration) {
    journal.record(&format!("pause-start:{:.3}", duration.as_secs_f64()));
    sleep(duration).await;
    journal.record(&format!("pause-done:{:.3}", duration.as_secs_f64()));
}

pub(crate) fn shutdown_channel() -> (watch::Sender<bool>, watch::Receiver<bool>) {
    watch::channel(false)
}

pub(crate) async fn wait_for_entry(journal: &Journal, entry: &str) {
    let recorded = timeout(Duration::from_secs(2), async {
        while journal.first_index(entry).is_none() {
            yield_now().await;
        }
    })
    .await;
    assert!(
        recorded.is_ok(),
        "the journal never recorded {entry}: {:?}",
        journal.entries()
    );
}

pub(crate) fn recording_callback(
    journal: &Journal,
    name: &'static str,
    outcome: Result<(), &'static str>,
) -> ResyncCallback<&'static str> {
    let recorded = journal.clone();
    Arc::new(move || {
        let task_journal = recorded.clone();
        Box::pin(async move {
            task_journal.record(name);
            outcome
        })
    })
}

pub(crate) struct RecordingTable {
    pub(crate) journal: Journal,
    pub(crate) failure: Option<&'static str>,
}

impl RecordingTable {
    pub(crate) fn recording(journal: &Journal) -> Self {
        Self {
            journal: journal.clone(),
            failure: None,
        }
    }

    fn outcome<T>(&self, value: T) -> Result<T, &'static str> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        Ok(value)
    }

    fn record_and<T>(&self, entry: &str, value: T) -> Result<T, &'static str> {
        self.journal.record(entry);
        self.outcome(value)
    }
}

impl TableActions for RecordingTable {
    type Row = i64;
    type Filter = &'static str;
    type CreateData = &'static str;
    type UpdateData = &'static str;
    type Failure = &'static str;

    fn create(
        &self,
        data: Self::CreateData,
    ) -> impl Future<Output = Result<Self::Row, Self::Failure>> + Send {
        ready(self.record_and(&format!("create:{data}"), 1))
    }

    fn create_many(
        &self,
        data: Vec<Self::CreateData>,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send {
        ready(self.record_and(&format!("create_many:{}", data.len()), data.len()))
    }

    fn update(
        &self,
        data: Self::UpdateData,
        filter: Self::Filter,
    ) -> impl Future<Output = Result<Option<Self::Row>, Self::Failure>> + Send {
        ready(self.record_and(&format!("update:{data}:{filter}"), Some(2)))
    }

    fn update_many(
        &self,
        data: Self::UpdateData,
        filter: Self::Filter,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send {
        ready(self.record_and(&format!("update_many:{data}:{filter}"), 0))
    }

    fn upsert(
        &self,
        filter: Self::Filter,
        data: Self::CreateData,
    ) -> impl Future<Output = Result<Self::Row, Self::Failure>> + Send {
        ready(self.record_and(&format!("upsert:{filter}:{data}"), 3))
    }

    fn delete(
        &self,
        filter: Self::Filter,
    ) -> impl Future<Output = Result<Option<Self::Row>, Self::Failure>> + Send {
        ready(self.record_and(&format!("delete:{filter}"), None))
    }

    fn delete_many(
        &self,
        filter: Self::Filter,
    ) -> impl Future<Output = Result<usize, Self::Failure>> + Send {
        ready(self.record_and(&format!("delete_many:{filter}"), 0))
    }
}

pub(crate) fn recording_publisher(journal: &Journal) -> PublishChange<&'static str> {
    publish_into(journal.clone(), None)
}

pub(crate) fn failing_publisher(
    journal: &Journal,
    error: &'static str,
) -> PublishChange<&'static str> {
    publish_into(journal.clone(), Some(error))
}

fn publish_into(journal: Journal, error: Option<&'static str>) -> PublishChange<&'static str> {
    Arc::new(move |object_type: &str| {
        let publish_journal = journal.clone();
        let name = object_type.to_owned();
        let outcome = error.map_or(Ok(()), Err);
        let future: PublishFuture<&'static str> = Box::pin(async move {
            publish_journal.record(&format!("publish:{name}"));
            outcome
        });
        future
    })
}

pub(crate) struct NoHooks;

impl ServiceHooks for NoHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn Error + Send + Sync)) {}
}

pub(crate) struct OpenGate;

impl BreakerGate for OpenGate {
    fn enter(&self, _name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Ok(BreakerAdmission::from(0))
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}

pub(crate) async fn redis_cache(server: &FakeServer, namespace: Option<&str>) -> RedisCache {
    redis_cache_at(&server.host(), server.port(), namespace).await
}

pub(crate) async fn redis_cache_at(host: &str, port: u16, namespace: Option<&str>) -> RedisCache {
    let overrides: RedisOverrides =
        serde_json::from_str(&format!(r#"{{"host": "{host}", "port": {port}}}"#))
            .expect("redis overrides from JSON");
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: namespace.map(str::to_owned),
        default_ttl: 60,
        redis_flush_size: None,
        default_major_version: 7,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(NoHooks),
        gate: Arc::new(OpenGate),
    };
    let env = BTreeMap::<String, String>::new();
    connect(options, &env).await.expect("a connected cache")
}

#[derive(Clone, Default)]
pub(crate) struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    pub(crate) fn capture() -> (Self, DefaultGuard) {
        let logs = Self::default();
        let guard = tracing_subscriber::fmt()
            .with_writer(logs.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .set_default();
        (logs, guard)
    }

    pub(crate) fn lines(&self) -> Vec<String> {
        let bytes = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8(bytes)
            .expect("UTF-8 log output")
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

impl io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Logs {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}
