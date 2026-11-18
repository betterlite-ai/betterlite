#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::mem::take;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use betterlite_db::queues::pod_lock::owner::PodId;
use betterlite_db::queues::pod_lock::{
    LockCache, LockCacheError, LockScript, NoPodLockEventSink, PodLockManager,
};
use betterlite_models::capture_rate::settings::{
    SpendCaptureProvider, SpendCaptureRateCheckSettings,
};
use betterlite_provider::billing::{
    BillingHttpGet, BillingHttpRequest, BillingHttpResponse, BillingTransportError,
};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use chrono::{NaiveDate, NaiveTime};
use rust_decimal::Decimal;
use secrecy::SecretString;
use uuid::Uuid;

use super::credentials::{BillingCredentialError, BillingCredentials};
use super::store::{CapturedSpendError, CapturedSpendRequest, CapturedSpendSource};
use super::{CaptureRateAlert, CaptureRatePublisher};

mod lease;
mod postgres;
mod report;

fn dec(text: &str) -> Decimal {
    text.parse().unwrap()
}

fn date(text: &str) -> NaiveDate {
    text.parse().unwrap()
}

fn midnight(day: &str) -> i64 {
    date(day).and_time(NaiveTime::MIN).and_utc().timestamp()
}

fn secret() -> SecretString {
    SecretString::from("sk-admin".to_owned())
}

fn settings(
    providers: &[SpendCaptureProvider],
    threshold: &str,
    lookback_days: i64,
    project_ids: &[&str],
) -> SpendCaptureRateCheckSettings {
    SpendCaptureRateCheckSettings {
        providers: providers.to_vec(),
        threshold: dec(threshold),
        lookback_days,
        openai_project_ids: project_ids.iter().map(|id| (*id).to_owned()).collect(),
    }
}

fn spend_days(pairs: &[(&str, &str)]) -> BTreeMap<String, Decimal> {
    pairs
        .iter()
        .map(|(day, spend)| ((*day).to_owned(), dec(spend)))
        .collect()
}

fn page(day: &str, amount: &str) -> String {
    format!(
        r#"{{"data":[{{"start_time":{},"results":[{{"amount":{{"value":{amount},"currency":"usd"}}}}]}}],"has_more":false}}"#,
        midnight(day)
    )
}

fn empty_page() -> String {
    r#"{"data":[],"has_more":false}"#.to_owned()
}

fn ok_page(body: String) -> BillingHttpResponse {
    BillingHttpResponse {
        status: 200,
        body: body.into_bytes(),
    }
}

#[derive(Default)]
struct Timeline {
    entries: Mutex<Vec<String>>,
}

impl Timeline {
    fn record(&self, entry: &str) {
        self.entries.lock().unwrap().push(entry.to_owned());
    }

    fn take(&self) -> Vec<String> {
        let mut entries = self.entries.lock().unwrap();
        take(&mut *entries)
    }
}

struct FakeCredentials {
    key: Option<SecretString>,
}

impl BillingCredentials for FakeCredentials {
    fn admin_key(&self) -> Result<Option<SecretString>, BillingCredentialError> {
        Ok(self.key.clone())
    }
}

struct FakeHttpGet {
    replies: Mutex<VecDeque<Result<BillingHttpResponse, BillingTransportError>>>,
    requests: Mutex<Vec<BillingHttpRequest>>,
}

impl FakeHttpGet {
    fn with(replies: Vec<Result<BillingHttpResponse, BillingTransportError>>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn take_requests(&self) -> Vec<BillingHttpRequest> {
        let mut requests = self.requests.lock().unwrap();
        take(&mut *requests)
    }
}

#[async_trait]
impl BillingHttpGet for FakeHttpGet {
    async fn get_billing(
        &self,
        request: BillingHttpRequest,
    ) -> Result<BillingHttpResponse, BillingTransportError> {
        self.requests.lock().unwrap().push(request);
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("a scripted billing reply")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SpendCall {
    providers: Vec<String>,
    start_date: NaiveDate,
    end_date: NaiveDate,
}

struct FakeSpend {
    replies: Mutex<VecDeque<Result<BTreeMap<String, Decimal>, CapturedSpendError>>>,
    requests: Mutex<Vec<SpendCall>>,
}

impl FakeSpend {
    fn with(replies: Vec<Result<BTreeMap<String, Decimal>, CapturedSpendError>>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn take_requests(&self) -> Vec<SpendCall> {
        let mut requests = self.requests.lock().unwrap();
        take(&mut *requests)
    }
}

#[async_trait]
impl CapturedSpendSource for FakeSpend {
    async fn captured_by_day(
        &self,
        request: &CapturedSpendRequest<'_>,
    ) -> Result<BTreeMap<String, Decimal>, CapturedSpendError> {
        self.requests.lock().unwrap().push(SpendCall {
            providers: request
                .providers
                .iter()
                .map(|provider| (*provider).to_owned())
                .collect(),
            start_date: request.start_date,
            end_date: request.end_date,
        });
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("a scripted captured-spend reply")
    }
}

struct RecordingPublisher {
    published: Mutex<Vec<(SpendCaptureProvider, Option<Decimal>)>>,
    timeline: Arc<Timeline>,
}

impl RecordingPublisher {
    fn new(timeline: Arc<Timeline>) -> Self {
        Self {
            published: Mutex::new(Vec::new()),
            timeline,
        }
    }

    fn take(&self) -> Vec<(SpendCaptureProvider, Option<Decimal>)> {
        let mut published = self.published.lock().unwrap();
        take(&mut *published)
    }
}

impl CaptureRatePublisher for RecordingPublisher {
    fn publish(&self, provider: SpendCaptureProvider, rate: Option<Decimal>) {
        self.published.lock().unwrap().push((provider, rate));
        self.timeline.record("publish");
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the alert sink is unavailable")]
struct AlertFailed;

struct RecordingAlert {
    messages: Mutex<Vec<String>>,
    failing: bool,
    timeline: Arc<Timeline>,
}

impl RecordingAlert {
    fn new(timeline: Arc<Timeline>) -> Self {
        Self {
            messages: Mutex::new(Vec::new()),
            failing: false,
            timeline,
        }
    }

    fn failing(timeline: Arc<Timeline>) -> Self {
        Self {
            messages: Mutex::new(Vec::new()),
            failing: true,
            timeline,
        }
    }

    fn take(&self) -> Vec<String> {
        let mut messages = self.messages.lock().unwrap();
        take(&mut *messages)
    }
}

#[async_trait]
impl CaptureRateAlert for RecordingAlert {
    type Error = AlertFailed;

    async fn alert(&self, message: &str) -> Result<(), AlertFailed> {
        self.messages.lock().unwrap().push(message.to_owned());
        self.timeline.record("alert");
        if self.failing {
            return Err(AlertFailed);
        }
        Ok(())
    }
}

enum Lease {
    Missing,
    Held(String),
}

struct FakeLeaseCache {
    lease: Mutex<Lease>,
    fail_set: AtomicBool,
    fail_get: AtomicBool,
    set_ttls: Mutex<Vec<i64>>,
    keys: Mutex<Vec<String>>,
    deletes: AtomicUsize,
    timeline: Arc<Timeline>,
}

impl FakeLeaseCache {
    fn missing(timeline: Arc<Timeline>) -> Self {
        Self::with_lease(Lease::Missing, timeline)
    }

    fn held(owner: &str, timeline: Arc<Timeline>) -> Self {
        Self::with_lease(Lease::Held(owner.to_owned()), timeline)
    }

    fn with_lease(lease: Lease, timeline: Arc<Timeline>) -> Self {
        Self {
            lease: Mutex::new(lease),
            fail_set: AtomicBool::new(false),
            fail_get: AtomicBool::new(false),
            set_ttls: Mutex::new(Vec::new()),
            keys: Mutex::new(Vec::new()),
            deletes: AtomicUsize::new(0),
            timeline,
        }
    }

    fn fail_set(&self) {
        self.fail_set.store(true, Ordering::SeqCst);
    }

    fn fail_get(&self) {
        self.fail_get.store(true, Ordering::SeqCst);
    }

    fn hold(&self, owner: &str) {
        *self.lease.lock().unwrap() = Lease::Held(owner.to_owned());
    }

    fn set_ttls(&self) -> Vec<i64> {
        self.set_ttls.lock().unwrap().clone()
    }

    fn keys(&self) -> Vec<String> {
        self.keys.lock().unwrap().clone()
    }

    fn deletes(&self) -> usize {
        self.deletes.load(Ordering::SeqCst)
    }
}

impl LockCache for FakeLeaseCache {
    fn get_owner(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, LockCacheError>> + Send {
        self.keys.lock().unwrap().push(key.to_owned());
        self.timeline.record("get_owner");
        if self.fail_get.load(Ordering::SeqCst) {
            return ready(Err(LockCacheError::UnexpectedReply {
                operation: "get_owner",
            }));
        }
        let owner = match &*self.lease.lock().unwrap() {
            Lease::Missing => None,
            Lease::Held(owner) => Some(owner.clone().into_bytes()),
        };
        ready(Ok(owner))
    }

    fn set_owner_if_absent(
        &self,
        key: &str,
        owner: &str,
        ttl_seconds: i64,
    ) -> impl Future<Output = Result<bool, LockCacheError>> + Send {
        self.keys.lock().unwrap().push(key.to_owned());
        self.set_ttls.lock().unwrap().push(ttl_seconds);
        self.timeline.record("set_owner_if_absent");
        if self.fail_set.load(Ordering::SeqCst) {
            return ready(Err(LockCacheError::UnexpectedReply {
                operation: "set_owner_if_absent",
            }));
        }
        let mut lease = self.lease.lock().unwrap();
        let acquired = match &*lease {
            Lease::Missing => {
                *lease = Lease::Held(owner.to_owned());
                true
            }
            Lease::Held(_) => false,
        };
        drop(lease);
        ready(Ok(acquired))
    }

    fn delete(&self, _key: &str) -> impl Future<Output = Result<i64, LockCacheError>> + Send {
        self.deletes.fetch_add(1, Ordering::SeqCst);
        self.timeline.record("delete");
        ready(Ok(0))
    }

    fn compare_and_delete_script(&self) -> Option<Arc<dyn LockScript>> {
        None
    }
}

fn pod_lock_manager(cache: Option<Arc<FakeLeaseCache>>) -> PodLockManager<FakeLeaseCache> {
    let pod_id = PodId::try_from(Uuid::new_v4()).unwrap();
    PodLockManager::new(cache, 900, Arc::new(NoPodLockEventSink), pod_id)
}

struct NoPorts;

impl OidcHttp for NoPorts {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the test has no OIDC transport".to_owned(),
        })
    }
}

impl AzureOidcTokens for NoPorts {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoPorts {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

fn secret_lookup() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoPorts),
        azure_tokens: Arc::new(NoPorts),
        clock: Arc::new(NoPorts),
        oidc_cache: OidcTokenCache::default(),
    }
}
