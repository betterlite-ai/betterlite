use std::collections::BTreeSet;
use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_config::sections::alert_settings::types::SlackAlertingArgs;

use crate::alert_dispatch::AlertLevel;
use crate::outage_alerts::OutageDetector;
use crate::outage_alerts::ports::{
    DeploymentWrite, OutageApiBase, OutageClock, OutageDeployment, OutageDeployments,
    OutageProviders, OutageSender, OutageStore, ProviderResolution, RegionWrite,
};
use crate::outage_alerts::record::{OutageRecord, RegionOutageRecord};

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("the fake port failed")]
pub(super) struct FakeFailure;

pub(super) struct SentAlert {
    pub(super) message: String,
    pub(super) level: AlertLevel,
}

#[derive(Debug, PartialEq)]
pub(super) enum StoreWrite {
    Deployment {
        key: String,
        record: OutageRecord,
        ttl_seconds: Option<u32>,
    },
    Region {
        key: String,
        record: RegionOutageRecord,
        ttl_seconds: Option<u32>,
    },
}

#[derive(Default)]
pub(super) struct StoreState {
    pub(super) deployment: Option<OutageRecord>,
    pub(super) region: Option<RegionOutageRecord>,
    pub(super) reads: Vec<String>,
    pub(super) writes: Vec<StoreWrite>,
    pub(super) write_attempts: Vec<String>,
    pub(super) fail_read: bool,
    pub(super) fail_write: bool,
}

trait StoredRecord: Clone + Sized {
    fn cached(state: &StoreState) -> Option<&Self>;
    fn store(state: &mut StoreState, record: Self);
    fn push_write(state: &mut StoreState, key: &str, record: &Self, ttl_seconds: Option<u32>);
}

impl StoredRecord for OutageRecord {
    fn cached(state: &StoreState) -> Option<&Self> {
        state.deployment.as_ref()
    }

    fn store(state: &mut StoreState, record: Self) {
        state.deployment = Some(record);
    }

    fn push_write(state: &mut StoreState, key: &str, record: &Self, ttl_seconds: Option<u32>) {
        state.writes.push(StoreWrite::Deployment {
            key: key.to_owned(),
            record: record.clone(),
            ttl_seconds,
        });
    }
}

impl StoredRecord for RegionOutageRecord {
    fn cached(state: &StoreState) -> Option<&Self> {
        state.region.as_ref()
    }

    fn store(state: &mut StoreState, record: Self) {
        state.region = Some(record);
    }

    fn push_write(state: &mut StoreState, key: &str, record: &Self, ttl_seconds: Option<u32>) {
        state.writes.push(StoreWrite::Region {
            key: key.to_owned(),
            record: record.clone(),
            ttl_seconds,
        });
    }
}

#[derive(Default)]
pub(super) struct FakeStore {
    state: Mutex<StoreState>,
}

impl FakeStore {
    pub(super) fn lock(&self) -> MutexGuard<'_, StoreState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    // One read path and one write path serve both cached record kinds.
    fn read<Record>(&self, key: &str) -> Result<Option<Record>, FakeFailure>
    where
        Record: StoredRecord,
    {
        let mut state = self.lock();
        state.reads.push(key.to_owned());
        if state.fail_read {
            return Err(FakeFailure);
        }
        let cached = Record::cached(&state).cloned();
        drop(state);
        Ok(cached)
    }

    fn write<Record>(
        &self,
        key: &str,
        record: &Record,
        ttl_seconds: Option<u32>,
    ) -> Result<(), FakeFailure>
    where
        Record: StoredRecord,
    {
        let mut state = self.lock();
        state.write_attempts.push(key.to_owned());
        if state.fail_write {
            return Err(FakeFailure);
        }
        Record::store(&mut state, record.clone());
        Record::push_write(&mut state, key, record, ttl_seconds);
        drop(state);
        Ok(())
    }
}

#[async_trait]
impl OutageStore for FakeStore {
    type Error = FakeFailure;

    async fn get_outage_record(&self, key: &str) -> Result<Option<OutageRecord>, FakeFailure> {
        self.read(key)
    }

    async fn set_outage_record(&self, write: DeploymentWrite<'_>) -> Result<(), FakeFailure> {
        self.write(write.key, write.record, write.ttl_seconds)
    }

    async fn get_region_outage_record(
        &self,
        key: &str,
    ) -> Result<Option<RegionOutageRecord>, FakeFailure> {
        self.read(key)
    }

    async fn set_region_outage_record(&self, write: RegionWrite<'_>) -> Result<(), FakeFailure> {
        self.write(write.key, write.record, write.ttl_seconds)
    }
}

pub(super) struct FakeDeployments {
    pub(super) found: Option<OutageDeployment>,
    pub(super) fail: bool,
    lookups: Mutex<Vec<String>>,
}

impl FakeDeployments {
    pub(super) fn new(deployment: OutageDeployment) -> Self {
        Self {
            found: Some(deployment),
            fail: false,
            lookups: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn lookups(&self) -> Vec<String> {
        self.lookups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl OutageDeployments for FakeDeployments {
    type Error = FakeFailure;

    fn deployment(&self, deployment_id: &str) -> Result<Option<OutageDeployment>, FakeFailure> {
        self.lookups
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(deployment_id.to_owned());
        if self.fail {
            return Err(FakeFailure);
        }
        Ok(self.found.clone())
    }
}

pub(super) struct FakeProviders {
    pub(super) result: Result<ProviderResolution, FakeFailure>,
    models: Mutex<Vec<String>>,
}

impl FakeProviders {
    pub(super) fn models(&self) -> Vec<String> {
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl OutageProviders for FakeProviders {
    type Error = FakeFailure;

    fn get_llm_provider(&self, model: &str) -> Result<ProviderResolution, FakeFailure> {
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.to_owned());
        self.result.clone()
    }
}

pub(super) struct FakeApiBase {
    pub(super) result: Result<Option<String>, FakeFailure>,
    calls: Mutex<Vec<(String, String)>>,
}

impl FakeApiBase {
    pub(super) fn calls(&self) -> Vec<(String, String)> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl OutageApiBase for FakeApiBase {
    type Error = FakeFailure;

    fn api_base(&self, model: &str, deployment_id: &str) -> Result<Option<String>, FakeFailure> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((model.to_owned(), deployment_id.to_owned()));
        self.result.clone()
    }
}

#[derive(Default)]
pub(super) struct FakeSender {
    pub(super) sent: Mutex<Vec<SentAlert>>,
    pub(super) fail: bool,
}

impl FakeSender {
    pub(super) fn sent(&self) -> Vec<SentAlert> {
        let sent = self.sent.lock().unwrap_or_else(PoisonError::into_inner);
        sent.iter()
            .map(|alert| SentAlert {
                message: alert.message.clone(),
                level: alert.level,
            })
            .collect()
    }
}

#[async_trait]
impl OutageSender for FakeSender {
    type Error = FakeFailure;

    async fn send_outage_alert(&self, message: &str, level: AlertLevel) -> Result<(), FakeFailure> {
        if self.fail {
            return Err(FakeFailure);
        }
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SentAlert {
                message: message.to_owned(),
                level,
            });
        Ok(())
    }
}

pub(super) struct FakeClock {
    pub(super) now: f64,
}

impl OutageClock for FakeClock {
    fn now_unix_seconds(&self) -> f64 {
        self.now
    }
}

pub(super) struct Fixture {
    pub(super) store: FakeStore,
    pub(super) deployments: FakeDeployments,
    pub(super) providers: FakeProviders,
    pub(super) api_base: FakeApiBase,
    pub(super) sender: FakeSender,
    pub(super) clock: FakeClock,
    pub(super) args: SlackAlertingArgs,
    pub(super) router: bool,
}

impl Fixture {
    pub(super) fn new(deployment: OutageDeployment) -> Self {
        Self {
            store: FakeStore::default(),
            deployments: FakeDeployments::new(deployment),
            providers: FakeProviders {
                result: Ok(ProviderResolution {
                    model: "unresolved".to_owned(),
                    provider: "unresolved".to_owned(),
                }),
                models: Mutex::new(Vec::new()),
            },
            api_base: FakeApiBase {
                result: Ok(Some("https://api.example.test".to_owned())),
                calls: Mutex::new(Vec::new()),
            },
            sender: FakeSender::default(),
            clock: FakeClock { now: NOW },
            args: SlackAlertingArgs::default(),
            router: true,
        }
    }

    pub(super) fn detector(
        &self,
    ) -> OutageDetector<
        '_,
        FakeStore,
        FakeDeployments,
        FakeProviders,
        FakeApiBase,
        FakeSender,
        FakeClock,
    > {
        OutageDetector {
            store: &self.store,
            deployments: self.router.then_some(&self.deployments),
            providers: &self.providers,
            api_base: &self.api_base,
            sender: &self.sender,
            clock: &self.clock,
            args: &self.args,
        }
    }
}

pub(super) fn model_deployment() -> OutageDeployment {
    let mut deployment = regional_deployment("openai", None, None, None, None);
    deployment.model = "gpt-4o".to_owned();
    deployment
}

pub(super) fn vertex_deployment() -> OutageDeployment {
    let mut deployment = regional_deployment("vertex_ai", None, Some("us-east1"), None, None);
    deployment.model = "gemini-1.5-pro".to_owned();
    deployment
}

pub(super) fn regional_deployment(
    provider: &str,
    region_name: Option<&str>,
    vertex_location: Option<&str>,
    aws_region_name: Option<&str>,
    watsonx_region_name: Option<&str>,
) -> OutageDeployment {
    OutageDeployment {
        model: "regional-model".to_owned(),
        custom_llm_provider: Some(provider.to_owned()),
        region_name: region_name.map(str::to_owned),
        vertex_location: vertex_location.map(str::to_owned),
        aws_region_name: aws_region_name.map(str::to_owned),
        watsonx_region_name: watsonx_region_name.map(str::to_owned),
    }
}

pub(super) const NOW: f64 = 400.0;

#[derive(Clone, Copy)]
pub(super) struct AlertFlags {
    pub(super) minor: bool,
    pub(super) major: bool,
}

pub(super) const NONE_SENT: AlertFlags = AlertFlags {
    minor: false,
    major: false,
};
pub(super) const MINOR_SENT: AlertFlags = AlertFlags {
    minor: true,
    major: false,
};
pub(super) const BOTH_SENT: AlertFlags = AlertFlags {
    minor: true,
    major: true,
};

pub(super) fn deployment_record(alerts: Vec<i64>, sent: AlertFlags) -> OutageRecord {
    OutageRecord {
        model_id: "dep-1".to_owned(),
        alerts,
        minor_alert_sent: sent.minor,
        major_alert_sent: sent.major,
        last_updated_at: NOW,
    }
}

pub(super) fn region_record(
    provider_region_id: &str,
    alerts: Vec<i64>,
    deployment_ids: &[&str],
    sent: AlertFlags,
) -> RegionOutageRecord {
    let ids: BTreeSet<String> = deployment_ids.iter().copied().map(str::to_owned).collect();
    RegionOutageRecord {
        provider_region_id: provider_region_id.to_owned(),
        alerts,
        minor_alert_sent: sent.minor,
        major_alert_sent: sent.major,
        last_updated_at: NOW,
        deployment_ids: ids,
    }
}
