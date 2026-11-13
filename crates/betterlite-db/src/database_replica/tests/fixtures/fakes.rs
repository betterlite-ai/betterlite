#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_util::future::{Either, select};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::sync::Notify;
use tracing::subscriber::{DefaultGuard, set_default};

use crate::database_refresh::handle::{PoolFuture, PoolTransport};
use crate::database_replica::availability::Availability;
use crate::database_replica::client::{Attribute, DatabaseClient, DatabaseLifecycle};
use crate::database_replica::routing::RoutingDatabase;

use super::logs::{CapturedLogs, capture_logs};

pub(crate) const WRITER_URL: &str = "postgresql://writer@primary.example:5432/betterlite";
pub(crate) const READER_URL: &str = "postgresql://reader@replica.example:5432/betterlite";
pub(crate) const MINTED_URL: &str = "postgresql://minted@replica.example:5432/betterlite";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Side {
    Writer,
    Reader,
}

impl Side {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Writer => "writer",
            Self::Reader => "reader",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, thiserror::Error)]
pub(crate) enum FakeError {
    #[default]
    #[error("connect failed")]
    Connect,
    #[error("disconnect failed")]
    Disconnect,
    #[error("attribute lookup failed")]
    Lookup,
    #[error("recreate failed")]
    Recreate,
    #[error("mint failed")]
    Mint,
    #[error("refresh failed")]
    Refresh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Connect,
    Disconnect,
    StartRefresh,
    StopRefresh,
}

impl Action {
    const fn event(self) -> &'static str {
        match self {
            Self::Connect => "connect",
            Self::Disconnect => "disconnect",
            Self::StartRefresh => "start_refresh",
            Self::StopRefresh => "stop_refresh",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Step {
    #[default]
    Succeed,
    Fail,
    Blocked,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum RecreateStep {
    #[default]
    Succeed,
    Stale,
    Fail,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FakeAttribute {
    pub(crate) side: Side,
    pub(crate) name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecreateCall {
    pub(crate) url: String,
    pub(crate) transport: Option<usize>,
    pub(crate) generation: Option<u64>,
}

#[derive(Default)]
pub(crate) struct EventLog {
    events: Mutex<Vec<String>>,
}

impl EventLog {
    pub(crate) fn record(&self, event: String) {
        self.events.lock().unwrap().push(event);
    }

    pub(crate) fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }

    pub(crate) fn has(&self, event: &str) -> bool {
        self.events
            .lock()
            .unwrap()
            .iter()
            .any(|entry| entry == event)
    }
}

#[derive(Default)]
pub(crate) struct Waiting {
    pub(crate) entered: Notify,
    pub(crate) hold: Notify,
}

async fn block(waiting: &Waiting) {
    waiting.entered.notify_one();
    waiting.hold.notified().await;
}

#[derive(Default)]
struct State {
    connected: bool,
    attributes: BTreeMap<String, FakeAttribute>,
    models: BTreeSet<String>,
    failing_lookups: BTreeSet<String>,
    connect: Step,
    connect_error: FakeError,
    disconnect: Step,
    disconnect_error: FakeError,
    recreate: RecreateStep,
    start_refresh: Step,
    stop_refresh: Step,
    token_refresh: bool,
    token_label: Option<&'static str>,
    minted: Option<Result<Option<SecretString>, FakeError>>,
    timeouts: Vec<Option<Duration>>,
    recreates: Vec<RecreateCall>,
}

pub(crate) struct FakeClient {
    side: Side,
    log: Arc<EventLog>,
    waiting: Arc<Waiting>,
    state: Mutex<State>,
}

impl FakeClient {
    pub(crate) fn new(side: Side, log: Arc<EventLog>, waiting: Arc<Waiting>) -> Self {
        Self {
            side,
            log,
            waiting,
            state: Mutex::new(State::default()),
        }
    }

    pub(crate) fn side(&self) -> Side {
        self.side
    }

    pub(crate) fn set_connected(&self, connected: bool) {
        self.state().connected = connected;
    }

    pub(crate) fn add_model(&self, name: &str) {
        self.add_attribute(name);
        self.state().models.insert(name.to_owned());
    }

    pub(crate) fn add_direct(&self, name: &str) {
        self.add_attribute(name);
    }

    fn add_attribute(&self, name: &str) {
        let attribute = FakeAttribute {
            side: self.side,
            name: name.to_owned(),
        };
        self.state().attributes.insert(name.to_owned(), attribute);
    }

    pub(crate) fn fail_lookup(&self, name: &str) {
        let mut state = self.state();
        state.failing_lookups.insert(name.to_owned());
    }

    pub(crate) fn script_connect(&self, step: Step) {
        self.state().connect = step;
    }

    pub(crate) fn fail_connect_with(&self, error: FakeError) {
        let mut state = self.state();
        state.connect = Step::Fail;
        state.connect_error = error;
    }

    pub(crate) fn script_disconnect(&self, step: Step) {
        self.state().disconnect = step;
    }

    pub(crate) fn fail_disconnect_with(&self, error: FakeError) {
        let mut state = self.state();
        state.disconnect = Step::Fail;
        state.disconnect_error = error;
    }

    pub(crate) fn script_recreate(&self, step: RecreateStep) {
        self.state().recreate = step;
    }

    pub(crate) fn script_start_refresh(&self, step: Step) {
        self.state().start_refresh = step;
    }

    pub(crate) fn script_stop_refresh(&self, step: Step) {
        self.state().stop_refresh = step;
    }

    pub(crate) fn with_token_refresh(&self, label: &'static str) {
        let mut state = self.state();
        state.token_refresh = true;
        state.token_label = Some(label);
    }

    pub(crate) fn with_minted(&self, minted: Result<Option<SecretString>, FakeError>) {
        self.state().minted = Some(minted);
    }

    pub(crate) fn timeouts(&self) -> Vec<Option<Duration>> {
        self.state().timeouts.clone()
    }

    pub(crate) fn recreates(&self) -> Vec<RecreateCall> {
        self.state().recreates.clone()
    }

    pub(crate) fn entered(&self) -> &Notify {
        &self.waiting.entered
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn failure_for(&self, action: Action) -> FakeError {
        match action {
            Action::Connect => self.state().connect_error,
            Action::Disconnect => self.state().disconnect_error,
            Action::StartRefresh | Action::StopRefresh => FakeError::Refresh,
        }
    }

    fn perform(&self, action: Action) -> PoolFuture<'_, (), FakeError> {
        Box::pin(async move {
            let side = self.side.name();
            self.log.record(format!("{side}:{}", action.event()));
            let step = match action {
                Action::Connect => self.state().connect,
                Action::Disconnect => self.state().disconnect,
                Action::StartRefresh => self.state().start_refresh,
                Action::StopRefresh => self.state().stop_refresh,
            };
            match step {
                Step::Succeed => {
                    self.settle(action);
                    Ok(())
                }
                Step::Fail => Err(self.failure_for(action)),
                Step::Blocked => {
                    block(&self.waiting).await;
                    Err(self.failure_for(action))
                }
            }
        })
    }

    fn settle(&self, action: Action) {
        match action {
            Action::Connect => self.state().connected = true,
            Action::Disconnect => self.state().connected = false,
            Action::StartRefresh | Action::StopRefresh => {}
        }
    }
}

impl DatabaseLifecycle for FakeClient {
    type Error = FakeError;

    fn is_connected(&self) -> bool {
        self.state().connected
    }

    fn connect(&self, timeout: Option<Duration>) -> PoolFuture<'_, (), FakeError> {
        self.state().timeouts.push(timeout);
        self.perform(Action::Connect)
    }

    fn disconnect(&self, timeout: Option<Duration>) -> PoolFuture<'_, (), FakeError> {
        self.state().timeouts.push(timeout);
        self.perform(Action::Disconnect)
    }

    fn recreate<'database>(
        &'database self,
        url: &'database SecretString,
        transport: Option<&'database PoolTransport>,
        expected_generation: Option<u64>,
    ) -> PoolFuture<'database, bool, FakeError> {
        Box::pin(async move {
            let side = self.side.name();
            self.log.record(format!("{side}:recreate"));
            let step = {
                let mut state = self.state();
                state.recreates.push(RecreateCall {
                    url: url.expose_secret().to_owned(),
                    transport: transport.and_then(|supplied| supplied.max_size),
                    generation: expected_generation,
                });
                state.recreate
            };
            match step {
                RecreateStep::Succeed => Ok(true),
                RecreateStep::Stale => Ok(false),
                RecreateStep::Fail => Err(FakeError::Recreate),
                RecreateStep::Blocked => {
                    block(&self.waiting).await;
                    Ok(true)
                }
            }
        })
    }

    fn start_refresh(&self) -> PoolFuture<'_, (), FakeError> {
        self.perform(Action::StartRefresh)
    }

    fn stop_refresh(&self) -> PoolFuture<'_, (), FakeError> {
        self.perform(Action::StopRefresh)
    }
}

impl DatabaseClient for FakeClient {
    type Attribute = FakeAttribute;

    fn attribute(&self, name: &str) -> Result<Option<Attribute<FakeAttribute>>, FakeError> {
        let side = self.side.name();
        self.log.record(format!("{side}:attribute:{name}"));
        let state = self.state();
        if state.failing_lookups.contains(name) {
            return Err(FakeError::Lookup);
        }
        let Some(attribute) = state.attributes.get(name) else {
            return Ok(None);
        };
        Ok(Some(if state.models.contains(name) {
            Attribute::Model(attribute.clone())
        } else {
            Attribute::Direct(attribute.clone())
        }))
    }

    fn token_refresh_enabled(&self) -> bool {
        self.state().token_refresh
    }

    fn token_label(&self) -> &'static str {
        self.state
            .lock()
            .unwrap()
            .token_label
            .unwrap_or("database token")
    }

    fn minted_url(&self) -> Result<Option<SecretString>, FakeError> {
        self.state().minted.clone().unwrap_or(Ok(None))
    }
}

pub(crate) struct Fixture {
    pub(crate) routing: RoutingDatabase<FakeClient>,
    pub(crate) log: Arc<EventLog>,
    pub(crate) captured: CapturedLogs,
    _guard: DefaultGuard,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        Self::with_reader_url(Some(READER_URL))
    }

    pub(crate) fn with_reader_url(url: Option<&str>) -> Self {
        let (captured, subscriber) = capture_logs();
        let guard = set_default(subscriber);
        let log = Arc::new(EventLog::default());
        let waiting = Arc::new(Waiting::default());
        let routing = RoutingDatabase {
            writer: FakeClient::new(Side::Writer, Arc::clone(&log), Arc::clone(&waiting)),
            reader: FakeClient::new(Side::Reader, Arc::clone(&log), Arc::clone(&waiting)),
            availability: Availability::default(),
            reader_url: url.map(|value| SecretString::from(value.to_owned())),
        };
        Self {
            routing,
            log,
            captured,
            _guard: guard,
        }
    }

    pub(crate) fn writer(&self) -> &FakeClient {
        &self.routing.writer
    }

    pub(crate) fn reader(&self) -> &FakeClient {
        &self.routing.reader
    }

    pub(crate) fn events(&self) -> Vec<String> {
        self.log.events()
    }

    pub(crate) fn has_event(&self, event: &str) -> bool {
        self.log.has(event)
    }

    pub(crate) fn warned(&self, message: &str) -> bool {
        self.captured.has("WARN", message)
    }

    pub(crate) fn informed(&self, message: &str) -> bool {
        self.captured.has("INFO", message)
    }
}

// drops the pending call the moment it demonstrably reached the blocked side, as a cancelled request would
pub(crate) async fn cancel_after_started<Operation, Output, Started>(
    call: Operation,
    signal: Started,
) where
    Operation: Future<Output = Output> + Send,
    Started: Future<Output = ()> + Send,
{
    let mut operation = Box::pin(call);
    let mut started = Box::pin(signal);
    match select(&mut operation, &mut started).await {
        Either::Left((_finished, _started)) => {
            panic!("a blocked call must not finish before it is cancelled")
        }
        Either::Right(((), _operation)) => {}
    }
}

pub(crate) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(crate) fn transport(max_size: usize) -> PoolTransport {
    PoolTransport {
        max_size: Some(max_size),
    }
}
