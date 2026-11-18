#![cfg(test)]

use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use betterlite_db::queues::pod_lock::owner::{PodId, lease_key};
use betterlite_db::queues::pod_lock::{
    LockCache, LockCacheError, LockScript, NoPodLockEventSink, PodLockManager,
};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_models::capture_rate::CHECK_JOB_ID;
use betterlite_models::capture_rate::report::CheckResult;
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
use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;

use super::credentials::{BillingCredentialError, BillingCredentials, SecretBillingCredentials};
use super::store::{CapturedSpendError, CapturedSpendRequest, CapturedSpendSource};
use super::{
    CaptureRateAlert, CaptureRateJob, CaptureRateJobError, CaptureRatePublisher, run_check,
    run_scheduled_check,
};

mod postgres;

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

fn ok_page(body: String) -> Result<BillingHttpResponse, BillingTransportError> {
    Ok(BillingHttpResponse {
        status: 200,
        body: body.into_bytes(),
    })
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
        std::mem::take(&mut *entries)
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
        std::mem::take(&mut *requests)
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
        std::mem::take(&mut *requests)
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
        std::mem::take(&mut *published)
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
        std::mem::take(&mut *messages)
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
        match &*lease {
            Lease::Missing => {
                *lease = Lease::Held(owner.to_owned());
                ready(Ok(true))
            }
            Lease::Held(_) => ready(Ok(false)),
        }
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
    fn now(&self) -> std::time::Instant {
        std::time::Instant::now()
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

#[tokio::test]
async fn a_missing_credential_skips_billing_and_captured_spend() {
    let timeline = Arc::new(Timeline::default());
    let credentials = FakeCredentials { key: None };
    let http = FakeHttpGet::with(Vec::new());
    let spend = FakeSpend::with(Vec::new());
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 3, &[]);
    let results = run_check(&job, &settings, Some(date("2026-11-18")))
        .await
        .unwrap();
    assert!(matches!(
        results.as_slice(),
        [CheckResult::CredentialMissing { env_var, .. }] if env_var == "OPENAI_ADMIN_KEY"
    ));
    assert_eq!(publisher.take(), vec![(SpendCaptureProvider::Openai, None)]);
    assert_eq!(
        alert.take(),
        vec![
            "Spend capture-rate check: OPENAI_ADMIN_KEY is not set, so the openai bill cannot be read. Set it or remove general_settings.spend_capture_rate_check. https://docs.litellm.ai/docs/proxy/spend_capture_rate"
                .to_owned()
        ]
    );
    assert!(http.take_requests().is_empty());
    assert!(spend.take_requests().is_empty());
    assert_eq!(timeline.take(), vec!["publish", "alert"]);
}

#[tokio::test]
async fn a_failed_bill_skips_the_captured_spend_query() {
    let timeline = Arc::new(Timeline::default());
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![Ok(BillingHttpResponse {
        status: 500,
        body: b"boom".to_vec(),
    })]);
    let spend = FakeSpend::with(Vec::new());
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 3, &[]);
    let results = run_check(&job, &settings, Some(date("2026-11-18")))
        .await
        .unwrap();
    assert!(matches!(
        results.as_slice(),
        [CheckResult::RequestFailed { detail, .. }] if detail == "HTTP 500: boom"
    ));
    assert_eq!(publisher.take(), vec![(SpendCaptureProvider::Openai, None)]);
    assert_eq!(
        alert.take(),
        vec![
            "Spend capture-rate check: could not read the openai bill (HTTP 500: boom). https://docs.litellm.ai/docs/proxy/spend_capture_rate"
                .to_owned()
        ]
    );
    assert!(spend.take_requests().is_empty());
}

#[tokio::test]
async fn a_transport_failure_becomes_a_request_failed_result() {
    let timeline = Arc::new(Timeline::default());
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![Err(BillingTransportError::Transport(
        TransportError::Closed,
    ))]);
    let spend = FakeSpend::with(Vec::new());
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 3, &[]);
    let results = run_check(&job, &settings, Some(date("2026-11-18")))
        .await
        .unwrap();
    let [CheckResult::RequestFailed { detail, .. }] = results.as_slice() else {
        panic!("expected a request failure");
    };
    assert!(detail.starts_with("request failed: "), "{detail}");
    assert!(spend.take_requests().is_empty());
}

#[tokio::test]
async fn a_report_covers_yesterday_and_the_preceding_lookback_days() {
    let timeline = Arc::new(Timeline::default());
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![ok_page(page("2026-11-16", "100"))]);
    let spend = FakeSpend::with(vec![Ok(spend_days(&[("2026-11-16", "45")]))]);
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 3, &["proj_a"]);
    let results = run_check(&job, &settings, Some(date("2026-11-18")))
        .await
        .unwrap();
    let [CheckResult::Report(report)] = results.as_slice() else {
        panic!("expected a report");
    };
    assert_eq!(report.provider, SpendCaptureProvider::Openai);
    assert_eq!(report.start_date, "2026-11-15");
    assert_eq!(report.end_date, "2026-11-17");
    assert_eq!(report.days.len(), 3);
    assert_eq!(report.captured_spend, dec("45"));
    assert_eq!(report.provider_spend, dec("100"));
    assert_eq!(report.capture_rate, Some(dec("0.45")));
    assert!(report.below_threshold);
    assert_eq!(
        publisher.take(),
        vec![(SpendCaptureProvider::Openai, Some(dec("0.45")))]
    );
    assert_eq!(
        alert.take(),
        vec![
            "Spend capture rate for openai is 45.0%, under the 90% threshold: LiteLLM captured $45.00 of the $100.00 openai bill for 2026-11-15 to 2026-11-17. Requests reach openai outside LiteLLM or cost tracking is dropping spend. https://docs.litellm.ai/docs/proxy/spend_capture_rate"
                .to_owned()
        ]
    );
    assert_eq!(timeline.take(), vec!["publish", "alert"]);
    let calls = spend.take_requests();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].providers,
        vec!["openai".to_owned(), "text-completion-openai".to_owned()]
    );
    assert_eq!(calls[0].start_date, date("2026-11-15"));
    assert_eq!(calls[0].end_date, date("2026-11-17"));
    let requests = http.take_requests();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .query
            .iter()
            .any(|(name, value)| name == "project_ids[]" && value == "proj_a")
    );
    assert!(
        requests[0]
            .query
            .iter()
            .any(|(name, value)| name == "start_time"
                && *value == midnight("2026-11-15").to_string())
    );
    assert!(
        requests[0]
            .query
            .iter()
            .any(|(name, value)| name == "end_time" && *value == midnight("2026-11-18").to_string())
    );
}

#[tokio::test]
async fn a_healthy_report_publishes_its_rate_without_alerting() {
    let timeline = Arc::new(Timeline::default());
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![ok_page(page("2026-11-17", "100"))]);
    let spend = FakeSpend::with(vec![Ok(spend_days(&[("2026-11-17", "95")]))]);
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 1, &[]);
    run_check(&job, &settings, Some(date("2026-11-18")))
        .await
        .unwrap();
    assert_eq!(
        publisher.take(),
        vec![(SpendCaptureProvider::Openai, Some(dec("0.95")))]
    );
    assert!(alert.take().is_empty());
    assert_eq!(timeline.take(), vec!["publish"]);
}

#[tokio::test]
async fn a_zero_bill_publishes_no_ratio_and_does_not_alert() {
    let timeline = Arc::new(Timeline::default());
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![ok_page(empty_page())]);
    let spend = FakeSpend::with(vec![Ok(spend_days(&[("2026-11-17", "5")]))]);
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 1, &[]);
    let results = run_check(&job, &settings, Some(date("2026-11-18")))
        .await
        .unwrap();
    let [CheckResult::Report(report)] = results.as_slice() else {
        panic!("expected a report");
    };
    assert_eq!(report.capture_rate, None);
    assert!(!report.below_threshold);
    assert_eq!(publisher.take(), vec![(SpendCaptureProvider::Openai, None)]);
    assert!(alert.take().is_empty());
}

#[tokio::test]
async fn duplicate_providers_are_checked_and_published_twice() {
    let timeline = Arc::new(Timeline::default());
    let credentials = FakeCredentials { key: None };
    let http = FakeHttpGet::with(Vec::new());
    let spend = FakeSpend::with(Vec::new());
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(
        &[SpendCaptureProvider::Openai, SpendCaptureProvider::Openai],
        "0.9",
        1,
        &[],
    );
    let results = run_check(&job, &settings, Some(date("2026-11-18")))
        .await
        .unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(
        publisher.take(),
        vec![
            (SpendCaptureProvider::Openai, None),
            (SpendCaptureProvider::Openai, None)
        ]
    );
    assert_eq!(alert.take().len(), 2);
    assert_eq!(
        timeline.take(),
        vec!["publish", "publish", "alert", "alert"]
    );
}

#[tokio::test]
async fn a_later_failure_interrupts_before_any_publication() {
    let timeline = Arc::new(Timeline::default());
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![
        ok_page(page("2026-11-17", "100")),
        ok_page(page("2026-11-17", "100")),
    ]);
    let unreadable = CapturedSpendError::UnreadableAmount {
        date: "2026-11-17".to_owned(),
        source: "not-a-decimal".parse::<Decimal>().unwrap_err(),
    };
    let spend = FakeSpend::with(vec![
        Ok(spend_days(&[("2026-11-17", "45")])),
        Err(unreadable),
    ]);
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(
        &[SpendCaptureProvider::Openai, SpendCaptureProvider::Openai],
        "0.9",
        1,
        &[],
    );
    let error = run_check(&job, &settings, Some(date("2026-11-18")))
        .await
        .unwrap_err();
    assert!(matches!(error, CaptureRateJobError::Spend(_)));
    assert!(publisher.take().is_empty());
    assert!(alert.take().is_empty());
    assert!(timeline.take().is_empty());
    assert_eq!(http.take_requests().len(), 2);
}

#[tokio::test]
async fn a_lookback_below_one_is_an_error_before_any_query() {
    let timeline = Arc::new(Timeline::default());
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(Vec::new());
    let spend = FakeSpend::with(Vec::new());
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 0, &[]);
    let error = run_check(&job, &settings, Some(date("2026-11-18")))
        .await
        .unwrap_err();
    assert!(matches!(error, CaptureRateJobError::Lookback));
    assert!(publisher.take().is_empty());
    assert!(http.take_requests().is_empty());
    assert!(spend.take_requests().is_empty());
}

#[tokio::test]
async fn an_alert_failure_propagates_after_the_publication() {
    let timeline = Arc::new(Timeline::default());
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![ok_page(page("2026-11-17", "100"))]);
    let spend = FakeSpend::with(vec![Ok(spend_days(&[("2026-11-17", "45")]))]);
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::failing(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 1, &[]);
    let error = run_check(&job, &settings, Some(date("2026-11-18")))
        .await
        .unwrap_err();
    assert!(matches!(error, CaptureRateJobError::Alert(_)));
    assert_eq!(publisher.take().len(), 1);
    assert_eq!(alert.take().len(), 1);
    assert_eq!(timeline.take(), vec!["publish", "alert"]);
}

#[tokio::test]
async fn an_absent_lock_manager_allows_the_alert() {
    let timeline = Arc::new(Timeline::default());
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![ok_page(page("2026-11-17", "100"))]);
    let spend = FakeSpend::with(vec![Ok(spend_days(&[("2026-11-17", "45")]))]);
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 1, &[]);
    run_scheduled_check(&job, &settings, None, Some(date("2026-11-18")))
        .await
        .unwrap();
    assert_eq!(publisher.take().len(), 1);
    assert_eq!(alert.take().len(), 1);
}

#[tokio::test]
async fn an_absent_cache_allows_the_alert() {
    let timeline = Arc::new(Timeline::default());
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![ok_page(page("2026-11-17", "100"))]);
    let spend = FakeSpend::with(vec![Ok(spend_days(&[("2026-11-17", "45")]))]);
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let manager = pod_lock_manager(None);
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 1, &[]);
    run_scheduled_check(&job, &settings, Some(&manager), Some(date("2026-11-18")))
        .await
        .unwrap();
    assert_eq!(alert.take().len(), 1);
    assert_eq!(timeline.take(), vec!["publish", "alert"]);
}

#[tokio::test]
async fn a_held_lock_publishes_then_suppresses_the_alert() {
    let timeline = Arc::new(Timeline::default());
    let cache = Arc::new(FakeLeaseCache::held("other-pod", Arc::clone(&timeline)));
    let manager = pod_lock_manager(Some(Arc::clone(&cache)));
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![ok_page(page("2026-11-17", "100"))]);
    let spend = FakeSpend::with(vec![Ok(spend_days(&[("2026-11-17", "45")]))]);
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 1, &[]);
    run_scheduled_check(&job, &settings, Some(&manager), Some(date("2026-11-18")))
        .await
        .unwrap();
    assert_eq!(publisher.take().len(), 1);
    assert!(alert.take().is_empty());
    assert_eq!(cache.set_ttls(), vec![900]);
    assert_eq!(cache.deletes(), 0);
    assert_eq!(
        cache.keys(),
        vec![
            lease_key(CHECK_JOB_ID),
            lease_key(CHECK_JOB_ID),
            lease_key(CHECK_JOB_ID),
        ]
    );
    assert_eq!(
        timeline.take(),
        vec!["publish", "set_owner_if_absent", "get_owner", "get_owner"]
    );
}

#[tokio::test]
async fn a_vanished_lock_after_a_failed_acquisition_allows_the_alert() {
    let timeline = Arc::new(Timeline::default());
    let cache = Arc::new(FakeLeaseCache::missing(Arc::clone(&timeline)));
    cache.fail_set();
    let manager = pod_lock_manager(Some(Arc::clone(&cache)));
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![ok_page(page("2026-11-17", "100"))]);
    let spend = FakeSpend::with(vec![Ok(spend_days(&[("2026-11-17", "45")]))]);
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 1, &[]);
    run_scheduled_check(&job, &settings, Some(&manager), Some(date("2026-11-18")))
        .await
        .unwrap();
    assert_eq!(alert.take().len(), 1);
    assert_eq!(cache.deletes(), 0);
    assert_eq!(
        timeline.take(),
        vec!["publish", "set_owner_if_absent", "get_owner", "alert"]
    );
}

#[tokio::test]
async fn an_unreadable_lock_after_a_failed_acquisition_allows_the_alert() {
    let timeline = Arc::new(Timeline::default());
    let cache = Arc::new(FakeLeaseCache::held("other-pod", Arc::clone(&timeline)));
    cache.fail_set();
    cache.fail_get();
    let manager = pod_lock_manager(Some(Arc::clone(&cache)));
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![ok_page(page("2026-11-17", "100"))]);
    let spend = FakeSpend::with(vec![Ok(spend_days(&[("2026-11-17", "45")]))]);
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 1, &[]);
    run_scheduled_check(&job, &settings, Some(&manager), Some(date("2026-11-18")))
        .await
        .unwrap();
    assert_eq!(alert.take().len(), 1);
    assert_eq!(cache.deletes(), 0);
}

#[tokio::test]
async fn a_reentrant_owner_allows_the_alert() {
    let timeline = Arc::new(Timeline::default());
    let cache = Arc::new(FakeLeaseCache::held("other-pod", Arc::clone(&timeline)));
    let manager = pod_lock_manager(Some(Arc::clone(&cache)));
    cache.hold(manager.pod_id.as_ref());
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![ok_page(page("2026-11-17", "100"))]);
    let spend = FakeSpend::with(vec![Ok(spend_days(&[("2026-11-17", "45")]))]);
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 1, &[]);
    run_scheduled_check(&job, &settings, Some(&manager), Some(date("2026-11-18")))
        .await
        .unwrap();
    assert_eq!(alert.take().len(), 1);
    assert_eq!(cache.deletes(), 0);
    assert_eq!(
        timeline.take(),
        vec!["publish", "set_owner_if_absent", "get_owner", "alert"]
    );
}

#[tokio::test]
async fn a_healthy_scheduled_check_does_not_claim_the_window() {
    let timeline = Arc::new(Timeline::default());
    let cache = Arc::new(FakeLeaseCache::missing(Arc::clone(&timeline)));
    let manager = pod_lock_manager(Some(Arc::clone(&cache)));
    let credentials = FakeCredentials {
        key: Some(secret()),
    };
    let http = FakeHttpGet::with(vec![ok_page(page("2026-11-17", "100"))]);
    let spend = FakeSpend::with(vec![Ok(spend_days(&[("2026-11-17", "95")]))]);
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 1, &[]);
    run_scheduled_check(&job, &settings, Some(&manager), Some(date("2026-11-18")))
        .await
        .unwrap();
    assert_eq!(publisher.take().len(), 1);
    assert!(alert.take().is_empty());
    assert!(cache.set_ttls().is_empty());
    assert!(cache.keys().is_empty());
    assert_eq!(timeline.take(), vec!["publish"]);
}

#[tokio::test]
async fn reads_the_admin_key_through_the_secret_lookup() {
    let lookup = secret_lookup();
    let present = BTreeMap::from([("OPENAI_ADMIN_KEY".to_owned(), "sk-admin".to_owned())]);
    let adapter = SecretBillingCredentials {
        lookup: &lookup,
        environment: &present,
    };
    let key = adapter.admin_key().unwrap().unwrap();
    assert_eq!(key.expose_secret(), "sk-admin");
    let absent = BTreeMap::new();
    let adapter = SecretBillingCredentials {
        lookup: &lookup,
        environment: &absent,
    };
    assert!(adapter.admin_key().unwrap().is_none());
    let boolean = BTreeMap::from([("OPENAI_ADMIN_KEY".to_owned(), "true".to_owned())]);
    let adapter = SecretBillingCredentials {
        lookup: &lookup,
        environment: &boolean,
    };
    assert!(adapter.admin_key().unwrap().is_none());
}

#[tokio::test]
async fn the_scheduled_check_still_publishes_without_a_credential() {
    let timeline = Arc::new(Timeline::default());
    let cache = Arc::new(FakeLeaseCache::missing(Arc::clone(&timeline)));
    let manager = pod_lock_manager(Some(Arc::clone(&cache)));
    let credentials = FakeCredentials { key: None };
    let http = FakeHttpGet::with(Vec::new());
    let spend = FakeSpend::with(Vec::new());
    let publisher = RecordingPublisher::new(Arc::clone(&timeline));
    let alert = RecordingAlert::new(Arc::clone(&timeline));
    let job = CaptureRateJob {
        spend: &spend,
        http_get: &http,
        credentials: &credentials,
        publisher: &publisher,
        alert: &alert,
    };
    let settings = settings(&[SpendCaptureProvider::Openai], "0.9", 1, &[]);
    run_scheduled_check(&job, &settings, Some(&manager), Some(date("2026-11-18")))
        .await
        .unwrap();
    assert_eq!(publisher.take(), vec![(SpendCaptureProvider::Openai, None)]);
    assert_eq!(alert.take().len(), 1);
    assert_eq!(cache.set_ttls(), vec![900]);
    assert_eq!(
        timeline.take(),
        vec!["publish", "set_owner_if_absent", "alert"]
    );
}
