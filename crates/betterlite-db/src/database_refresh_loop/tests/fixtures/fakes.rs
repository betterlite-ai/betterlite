use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Utc};
use percent_encoding::utf8_percent_encode;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::sync::Notify;

use crate::database_refresh::endpoint::DatabaseUrlSlot;
use crate::database_refresh::handle::{PoolFuture, PoolHandle, PoolProvider, PoolTransport};
use crate::database_refresh_loop::clock::RefreshClock;
use crate::database_tokens::endpoint::QUOTE_SET;
use crate::database_tokens::provider::{
    AzureProviderFactory, AzureTokenProvider, RdsTokenProvider,
};
use crate::rds_credentials::RdsEndpoint;
use crate::rds_credentials::error::RdsCredentialsError;

pub(crate) const EXPIRY_900: &str = "?X-Amz-Date=20240101T000000Z&X-Amz-Expires=900";
pub(crate) const EXPIRY_100: &str = "?X-Amz-Date=20240101T000000Z&X-Amz-Expires=100";
// a provider returns the token in wire form, so a minted one is already percent-encoded
pub(crate) const MINTED_900: &str = "%3FX-Amz-Date%3D20240101T000000Z%26X-Amz-Expires%3D900";
pub(crate) const UNPARSABLE_TOKEN_URL: &str = "postgresql://user:password@db.example:5432/litellm";

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

pub(crate) fn naive(text: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f").expect("a test timestamp")
}

pub(crate) fn utc(text: &str) -> DateTime<Utc> {
    naive(text).and_utc()
}

pub(crate) fn token_url(token: &str) -> String {
    format!(
        "postgresql://user:{}@db.example:5432/litellm",
        utf8_percent_encode(token, QUOTE_SET)
    )
}

pub(crate) fn slot_url(slot: &DatabaseUrlSlot) -> Option<String> {
    let current = slot.snapshot();
    current.as_ref().map(|url| url.expose_secret().to_owned())
}

pub(crate) struct FakeClock {
    pub(crate) now: Mutex<DateTime<Utc>>,
}

impl RefreshClock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        *lock(&self.now)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum FakeError {
    #[error("connect failed")]
    Connect,
    #[error("mint failed")]
    Mint,
    #[error("callback failed")]
    Callback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectStep {
    Succeed,
    Fail,
    Blocked,
}

#[derive(Default)]
pub(crate) struct FakeScript {
    events: Mutex<Vec<String>>,
    builds: Mutex<Vec<String>>,
    connect_steps: Mutex<BTreeMap<usize, ConnectStep>>,
    connect_gates: Mutex<BTreeMap<usize, Arc<Notify>>>,
    next_id: AtomicUsize,
}

impl FakeScript {
    pub(crate) fn record(&self, event: String) {
        lock(&self.events).push(event);
    }

    pub(crate) fn build_urls(&self) -> Vec<String> {
        lock(&self.builds).clone()
    }

    pub(crate) fn has_event(&self, wanted: &str) -> bool {
        lock(&self.events).iter().any(|event| event == wanted)
    }

    pub(crate) fn event_count(&self, wanted: &str) -> usize {
        lock(&self.events)
            .iter()
            .filter(|event| *event == wanted)
            .count()
    }

    pub(crate) fn script_connect(&self, id: usize, step: ConnectStep) {
        lock(&self.connect_steps).insert(id, step);
    }

    pub(crate) fn gate_connect(&self, id: usize, gate: Arc<Notify>) {
        lock(&self.connect_gates).insert(id, gate);
    }

    fn next_id(&self) -> usize {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    fn connect_step(&self, id: usize) -> ConnectStep {
        lock(&self.connect_steps)
            .get(&id)
            .copied()
            .unwrap_or(ConnectStep::Succeed)
    }

    fn connect_gate(&self, id: usize) -> Option<Arc<Notify>> {
        lock(&self.connect_gates).get(&id).cloned()
    }
}

async fn wait_for_gate(gate: Option<Arc<Notify>>) {
    if let Some(ready) = gate {
        ready.notified().await;
    }
}

pub(crate) struct FakeHandle {
    pub(crate) id: usize,
    script: Arc<FakeScript>,
    connected: AtomicBool,
}

impl PoolHandle for FakeHandle {
    type Error = FakeError;

    fn connect(&self, _timeout: Option<Duration>) -> PoolFuture<'_, (), Self::Error> {
        Box::pin(async move {
            self.script.record(format!("connect:{}", self.id));
            match self.script.connect_step(self.id) {
                ConnectStep::Succeed => {}
                ConnectStep::Fail => return Err(FakeError::Connect),
                ConnectStep::Blocked => wait_for_gate(self.script.connect_gate(self.id)).await,
            }
            self.connected.store(true, Ordering::SeqCst);
            Ok(())
        })
    }

    fn close(&self) {
        self.script.record(format!("close:{}", self.id));
        self.connected.store(false, Ordering::SeqCst);
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn query<'borrowed>(
        &'borrowed self,
        sql: &'borrowed str,
    ) -> PoolFuture<'borrowed, u64, Self::Error> {
        Box::pin(async move {
            self.script.record(format!("query:{sql}"));
            Ok(1)
        })
    }
}

pub(crate) struct FakeProvider {
    pub(crate) script: Arc<FakeScript>,
}

impl PoolProvider for FakeProvider {
    type Handle = FakeHandle;
    type Error = FakeError;

    fn build(
        &self,
        url: &SecretString,
        _transport: Option<&PoolTransport>,
    ) -> Result<FakeHandle, FakeError> {
        let id = self.script.next_id();
        self.script.record(format!("build:{id}"));
        lock(&self.script.builds).push(url.expose_secret().to_owned());
        Ok(FakeHandle {
            id,
            script: Arc::clone(&self.script),
            connected: AtomicBool::new(false),
        })
    }
}

pub(crate) struct FakeRdsProvider;

impl RdsTokenProvider for FakeRdsProvider {
    fn mint(&self, _endpoint: &RdsEndpoint<'_>) -> Result<SecretString, RdsCredentialsError> {
        Ok(secret("minted-token"))
    }
}

pub(crate) struct UnusedAzureProvider;

impl AzureTokenProvider for UnusedAzureProvider {
    type Error = FakeError;

    fn token(&self) -> Result<SecretString, Self::Error> {
        Err(FakeError::Mint)
    }
}

pub(crate) struct UnusedAzureFactory;

impl AzureProviderFactory for UnusedAzureFactory {
    type Provider = UnusedAzureProvider;
    type Error = FakeError;

    fn build(&self, _scope: &str) -> Result<Arc<Self::Provider>, Self::Error> {
        Ok(Arc::new(UnusedAzureProvider))
    }
}
